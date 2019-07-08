use std::fs::File;
use std::io;
use std::io::{BufReader, BufRead, Write, Read, Seek, SeekFrom};
use sysinfo::SystemExt;
use byteorder::{ByteOrder, LittleEndian};
use std::{thread, time};
use chrono::Utc;
use std::cmp::min;
//use nix::sys::{ptrace, wait, signal};
//use nix::unistd::Pid;

// only interested in segments with at least 1000 pages
const SEGMENT_THRESHOLD: usize = 1000 * 4096;

// hmm, this is hard-coding 64-bit systems in here...
// TODO: detect user space end from current system address size.
// https://lwn.net/Articles/738975/
const USERSPACE_END: usize = 0xffff800000000000;

const PAGE_SIZE: usize = 4096;

// One bit per page; bit number if PFN.
const IDLE_BITMAP_PATH: &str = "/sys/kernel/mm/page_idle/bitmap";

const KPAGEFLAGS_PATH: &str = "/proc/kpageflags";
// Without a process ID means get the memory activity for the whole host.
// Note it doesn't analyze page contents, only type and activity.
pub fn get_host_memory(sleep: u64, inspect_ram: bool) -> Result<super::ProcessMemory, std::io::Error> {
    let physical_segments = get_physical_segments()?;
    set_idlemap(&physical_segments)?;
    //ptrace::cont(nix_pid, None);
    debug!("Sleeping {} seconds", sleep);
    thread::sleep(time::Duration::from_secs(sleep));
    let snapshot_time = Utc::now();
    //signal::kill(nix_pid, signal::Signal::SIGSTOP);
    let idlemap = load_idlemap(&physical_segments)?;
    Ok(super::ProcessMemory {
        timestamp: snapshot_time,
        segments: physical_segments.iter().map(|segment|
            super::Segment {
                addr_start: 0,
                page_flags: get_kpageflags(segment).unwrap().into_iter().enumerate().map(|(pfn_offset, pfn_flags)| {
                    let active_page_add = get_active_add(((segment.start_address / PAGE_SIZE) + pfn_offset) as u64, &idlemap);
                    let zero_page_add: u64 = match inspect_ram {
                        true => match get_pfn_content((segment.start_address / PAGE_SIZE) + pfn_offset) {
                            Ok(content) => match content.iter().all(|&x| x == 0) {
                                true => 1 << super::ZERO_PAGE_BIT,
                                false => 0,
                            },
                            Err(e) => panic!("Got error: {:?}", e),
                        },
                        false => 0
                    };
                    (pfn_flags & !(1 << super::ACTIVE_PAGE_BIT))
                        + active_page_add
                        + zero_page_add
                }).collect(),
            }
        ).collect(),
    })
}

pub fn get_memory(pid: i32, sleep: u64) -> Result<super::ProcessMemory, std::io::Error> {
    //let nix_pid = Pid::from_raw(pid);
    //ptrace::attach(nix_pid);
    //wait::waitpid(nix_pid, None);
    //ptrace::detach(nix_pid);
    let physical_segments = get_physical_segments()?;
    set_idlemap(&physical_segments)?;
    //ptrace::cont(nix_pid, None);
    debug!("Sleeping {} seconds", sleep);
    thread::sleep(time::Duration::from_secs(sleep));
    let snapshot_time = Utc::now();
    //signal::kill(nix_pid, signal::Signal::SIGSTOP);
    let idlemap = load_idlemap(&physical_segments)?;
    let segments: Vec<Segment> = get_virtual_segments(pid)?.into_iter()
        .filter(|s| s.start_address < USERSPACE_END && s.size >= SEGMENT_THRESHOLD)
        .collect();
    debug!("Process has {} (filtered segments", segments.len());
    let mut process_memory = super::ProcessMemory {
        timestamp: snapshot_time,
        segments : Vec::with_capacity(segments.len()),
    };
    let start_time = Utc::now();
    for segment in segments {
        let pagemap: Vec<u64> = get_pagemap(pid, &segment)?;
        debug!("Pagemap for segment at {} with size {} has len {}", segment.start_address, segment.size, pagemap.len());
        //let all_page_data = get_page_content(pid, segment.start_address)?;
        let mut data_slice: Option<Vec<u8>> = None;
        let mut data_slice_offset = 0;
        let page_flags: Vec<u64> = pagemap.iter().enumerate().map(|(page_idx, pagemap_word)|
            if pagemap_word & 1 << 63 == 0 {
                data_slice = None; //end of contiguous; clear for next mapped page.
                return pagemap_word.clone();
            } else {
                if pagemap_word & 1 << 62 != 0 {
                    data_slice = None; //end of contiguous; clear for next mapped page.
                    return pagemap_word.clone();

                } else {
                    if data_slice == None {
                        let page_range = contiguous_mapped_length(&pagemap[page_idx..]);
                        assert!(page_range > 0); // debugging; remove once happy with algorithm.
                        data_slice_offset = 0;
                        data_slice = Some(get_page_content(pid, segment.start_address + (page_idx * PAGE_SIZE), page_range).unwrap());
                    }
                    let page_data = match data_slice {
                        None => panic!("We were supposed to pre-read this but didn't...."),
                        Some(ref data_slice) => data_slice[(data_slice_offset* PAGE_SIZE)..((data_slice_offset+1) * PAGE_SIZE)].to_vec(),
                    };
                    data_slice_offset += 1;

                    let zero_page_add: u64 = match page_data.iter().all(|&x| x == 0) {
                        true => 1 << super::ZERO_PAGE_BIT,
                        false => 0,
                    };

                    // Bits 0-54  page frame number (PFN) if present
                    let active_page_add = get_active_add(pagemap_word & 0x7FFFFFFFFFFFFF, &idlemap);
                    // Zero the PFN; were going to use it to store other data resembling kpageflags
                    return (pagemap_word & !0x7FFFFFFFFFFFFF)
                            + zero_page_add + active_page_add;
                }
            }
        ).collect();
        process_memory.segments.push(super::Segment {
            addr_start: segment.start_address,
            page_flags: page_flags,
        });
    }
    debug!("Finished dumping segments in {} ms", (Utc::now() - start_time).num_milliseconds());
    Ok(process_memory)
}

fn get_active_add(pfn: u64, idlemap: &[u8]) -> u64 {
    return match idlemap[pfn as usize / 8] & 1 << pfn % 8 == 0 {
        true => 1 << super::ACTIVE_PAGE_BIT,
        false => 0,
    };
}

// Given an array slice of pagemap entries, where the starting element is a resident entry,
// returns how long the contiguous segment of resident entries is.
fn contiguous_mapped_length(pagemap: &[u64]) -> usize {
    for (idx, entry) in pagemap.iter().enumerate() {
        if entry & 1 << 63 == 0 {
            assert!(idx > 0); // we only expect this to be called when at a valid slice.
            return idx;
        }
    }
    return pagemap.len() // they're all valid!
}

struct Segment {
    pub start_address: usize,
    pub size: usize,
}

fn get_pagemap(pid: i32, segment: &Segment) -> std::io::Result<Vec<u64>> {
    return read_segment_data_from_file(segment, &format!("/proc/{}/pagemap", pid));
}

fn get_kpageflags(segment: &Segment) -> std::io::Result<Vec<u64>> {
    return read_segment_data_from_file(segment, KPAGEFLAGS_PATH);
}

fn read_segment_data_from_file(segment: &Segment, file_path: &str) -> std::io::Result<Vec<u64>> {
    let start_time = Utc::now();
    assert_eq!(segment.start_address % PAGE_SIZE, 0);
    // This is why we need to run the program as root
    // https://www.kernel.org/doc/Documentation/vm/pagemap.txt
    let mut file = File::open(file_path)?;
    // 64-bits = 8 bytes per page
    file.seek(SeekFrom::Start((segment.start_address / PAGE_SIZE) as u64 * 8))?;
    let mut data_bytes: Vec<u8> = Vec::with_capacity((segment.size / PAGE_SIZE) * 8);
    data_bytes.resize((segment.size / PAGE_SIZE ) * 8, 0);
    file.read_exact(data_bytes.as_mut_slice())?;
    assert_eq!(data_bytes.len() % 8, 0);
    let mut data_words: Vec<u64> = Vec::with_capacity(data_bytes.len() / 8);
    data_words.resize(data_bytes.len() / 8, 0);
    LittleEndian::read_u64_into(&data_bytes, &mut data_words);
    debug!("Loaded {} in {} ms", file_path, (Utc::now() - start_time).num_milliseconds());
    Ok(data_words)
}

fn get_virtual_segments(pid: i32) -> Result<Vec<Segment>, io::Error> {
    let file = File::open(format!("/proc/{}/maps", pid))?;
    Ok(parse_segment_addresses(
            BufReader::new(file).lines()
            .map(|line| line.unwrap()).collect()))
}

fn get_physical_segments() -> Result<Vec<Segment>, io::Error> {
    let file = File::open("/proc/iomem")?;
    Ok(parse_segment_addresses(
            BufReader::new(file).lines()
            .map(|line| line.unwrap())
            .filter(|line| line.contains("System RAM"))
            .collect()))
}

fn parse_segment_addresses(lines: Vec<String>) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    for line in lines {
        if let Ok((a, b)) = scan_fmt!(&line, "{x}-{x}", [hex usize], [hex usize]) {
            segments.push(Segment {
                start_address: a,
                size: b - a + 1,
            })
        } else {
            error!("Unable to parse maps line: {}", line);
        }
    }
    return segments;
}

fn set_idlemap(physical_segments: &[Segment]) -> std::io::Result<()> {
    // segment size = 8 * bytes-written * PAGE_SIZE
    let start_time = Utc::now();
    for segment in physical_segments {
        let idle_bitmap_data: Vec<u8> = vec![0xff; 4096];
        let mut file = File::create(IDLE_BITMAP_PATH)?;
        // extra divide and multiple to round down to starting 8-byte boundary.
        file.seek(SeekFrom::Start(((((segment.start_address / PAGE_SIZE) / 8) / 8 ) * 8) as u64))?;
        let mut write_counter: usize = 0;
        let bytes_to_write: usize = ((segment.size / PAGE_SIZE) + 8 - 1) / 8;
        while write_counter < bytes_to_write  {
            // is there a better way to round up to closest multiple of 8?
            let to_write = ((min(idle_bitmap_data.len(), bytes_to_write - write_counter) - 1) | 0x7) + 1;
            write_counter += file.write(&idle_bitmap_data[0..to_write])?;
        }
    }
    debug!("Idlemap set in {} ms", (Utc::now() - start_time).num_milliseconds());
    Ok(())
}

// There are fewer idlemap bytes than the vector. Rather than some sparse vector data
// structure I'll create a larger vector and keep the rest of the bits as 0.
// It's a bit (10%?) wasteful but keeps the logic simpler.
fn load_idlemap(physical_segments: &[Segment]) -> std::io::Result<Vec<u8>> {
    let start_time = Utc::now();
    let idlemap_size = (physical_segments.last().unwrap().start_address + physical_segments.last().unwrap().size + 1) / (PAGE_SIZE * 8);
    let mut idlemap: Vec<u8> = Vec::with_capacity(idlemap_size);
    idlemap.resize(idlemap_size, 0);
    let mut file = File::open(IDLE_BITMAP_PATH)?;
    let mut read_counter = 0;
    for segment in physical_segments {
        let offset = (((segment.start_address / PAGE_SIZE) / 8) / 8 ) * 8;
        let bytes = ((segment.size/ PAGE_SIZE) + 8 - 1) / 8;
        let to_read = ((bytes - 1) | 0x7) + 1;
        file.seek(SeekFrom::Start(offset as u64))?;
        file.read_exact(&mut idlemap[offset..(offset+to_read)])?;
        read_counter += to_read;
    }
    debug!("Idlemap of {} bytes loaded to vec with size {} in {} ms", read_counter, idlemap.len(), (Utc::now() - start_time).num_milliseconds());
    Ok(idlemap)
}

fn get_page_content(pid: i32, page_addr_start: usize, pages: usize) -> std::io::Result<Vec<u8>> {
    let mut mem_file = File::open(format!("/proc/{}/mem", pid))?;
    mem_file.seek(SeekFrom::Start(page_addr_start as u64))?;
    let mut mem: Vec<u8> = Vec::with_capacity(pages * PAGE_SIZE);
    mem.resize(pages * PAGE_SIZE, 0); // why do I have to do this...?
    mem_file.read_exact(mem.as_mut_slice())?;
    Ok(mem)
}

fn get_pfn_content(pfn: usize) -> std::io::Result<Vec<u8>> {
    let mut mem_file = File::open("/dev/mem")?;
    mem_file.seek(SeekFrom::Start((pfn * PAGE_SIZE) as u64))?;
    let mut mem: Vec<u8> = Vec::with_capacity(PAGE_SIZE);
    mem.resize(PAGE_SIZE, 0); // why do I have to do this...?
    mem_file.read_exact(mem.as_mut_slice())?;
    Ok(mem)
}

fn system_ram_pages() -> usize {
    system_ram_bytes() / PAGE_SIZE
}

fn system_ram_bytes() -> usize {
    // this seems to take 60 ms. :'-(
    sysinfo::System::new().get_total_memory() as usize * 1024
}
