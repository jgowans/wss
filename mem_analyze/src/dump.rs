use std::error;
use std::fs::File;
use std::io;
use std::io::{BufReader, BufRead, Write, Read, Seek, SeekFrom};
use sysinfo::SystemExt;
use byteorder::{ByteOrder, LittleEndian};
use std::{thread, time};
use chrono::{DateTime, Utc};

// only interested in segments with at least 1000 pages
const SEGMENT_THRESHOLD: usize = 1000 * 4096;

// hmm, this is hard-coding 64-bit systems in here...
// TODO: detect user space end from current system address size.
// https://lwn.net/Articles/738975/
const USERSPACE_END: usize = 0xffff800000000000;

const PAGE_SIZE: usize = 4096;

// One bit per page; bit number if PFN.
const IDLE_BITMAP_PATH: &str = "/sys/kernel/mm/page_idle/bitmap";


pub fn get_memory(pid: i32) -> Result<super::ProcessMemory, std::io::Error> {
    set_idlemap()?;
    //thread::sleep(time::Duration::from_secs(1));
    // TODO: sleep here
    let snapshot_time = Utc::now();
    let idlemap = load_idlemap()?;
    let segments: Vec<Segment> = get_segments(pid)?.into_iter()
        .filter(|s| s.start_address < USERSPACE_END && s.size >= SEGMENT_THRESHOLD)
        .collect();
    let mut process_memory = super::ProcessMemory {
        timestamp: snapshot_time,
        segments : Vec::with_capacity(segments.len()),
    };
    let mut total_pages = 0;
    let mut mapped_pages = 0;
    for segment in segments {
        let pagemap: Vec<u64> = get_pagemap(pid, &segment)?;
        println!("Pagemap: {:x?}", pagemap);
        println!("Pagemap for segment at {} with size {} has len {}", segment.start_address, segment.size, pagemap.len());
        //let all_page_data = get_page_content(pid, segment.start_address)?;
        let mut data_slice: Option<Vec<u8>> = None;
        let mut data_slice_offset = 0;
        let mut pages: Vec<super::Page> = Vec::new();
        for (page_idx, pagemap_word) in pagemap.iter().enumerate() {
            total_pages += 1;
            // Bits 0-54  page frame number (PFN) if present
            let pfn = pagemap_word & 0x7FFFFFFFFFFFFF;
            let page_status: super::PageStatus;
            let page_data: Option<Vec<u8>>;
            if pfn == 0 {
                page_status = super::PageStatus::Unmapped;
                page_data = None;
            } else {
                mapped_pages += 1;
                if pagemap_word & 1 << 62 != 0 {
                    page_status = super::PageStatus::Swapped;
                    page_data = None;

                } else {
                    if data_slice == None {
                        println!("About to check page range");
                        let page_range = contiguous_mapped_length(&pagemap[page_idx..]);
                        data_slice_offset = 0;
                        data_slice = Some(get_page_content(pid, segment.start_address, page_range)?);
                        println!("Page range: {}", page_range);
                    }
                    if idlemap[pfn as usize / 8] & 1 << pfn % 8 == 0 {
                        page_status = super::PageStatus::Active;
                    } else {
                        page_status = super::PageStatus::Idle;
                    }
                    page_data = Some(match data_slice {
                        None => panic!("We were supposed to pre-read this but didn't...."),
                        Some(ref data_slice) => data_slice[(data_slice_offset* PAGE_SIZE)..((data_slice_offset+1) * PAGE_SIZE)].to_vec(),
                    });
                }
            }
            pages.push(super::Page {
                status: page_status,
                data: page_data,
            })
        }
        process_memory.segments.push(super::VirtualSegment {
            virtual_addr_start: segment.start_address,
            pages: pages,
        });
        println!("Segment: {} size {}", segment.start_address, segment.size);
        println!("Total pages {} mapped pages {}", total_pages, mapped_pages);

    }
    Ok(process_memory)
}

fn contiguous_mapped_length(pagemap: &[u64]) -> usize {
    let mut length = 0;
    while length < pagemap.len() {
        length += 1;
        if pagemap[length] & 0x7FFFFFFFFFFFFF == 0 {
            break;
        }
    }
    length
}

struct Segment {
    pub start_address: usize,
    pub size: usize,
}

fn get_pagemap(pid: i32, segment: &Segment) -> std::io::Result<Vec<u64>> {
    println!("Starting to get pagemap");
    assert_eq!(segment.start_address % PAGE_SIZE, 0);
    // This is why we need to run the program as root
    // https://www.kernel.org/doc/Documentation/vm/pagemap.txt
    let mut file = File::open(format!("/proc/{}/pagemap", pid))?;
    // 64-bits = 8 bytes per page
    file.seek(SeekFrom::Start((segment.start_address / PAGE_SIZE) as u64 * 8))?;
    let mut pagemap: Vec<u8> = Vec::with_capacity((segment.size / PAGE_SIZE) * 8);
    pagemap.resize((segment.size / PAGE_SIZE ) * 8, 0);
    file.read_exact(pagemap.as_mut_slice())?;
    assert_eq!(pagemap.len() % 8, 0);
    let mut pagemap_words: Vec<u64> = Vec::with_capacity(pagemap.len() / 8);
    pagemap_words.resize(pagemap.len() / 8, 0);
    LittleEndian::read_u64_into(&pagemap, &mut pagemap_words);
    println!("Finished getting pagemap");
    Ok(pagemap_words)
}

fn get_segments(pid: i32) -> Result<Vec<Segment>, io::Error> {
    let mut segments: Vec<Segment> = Vec::new();
    let file = File::open(format!("/proc/{}/maps", pid))?;
    for line in BufReader::new(file).lines() {
        let line = line?;
        if let Ok((a, b)) = scan_fmt!(&line, "{x}-{x}", [hex usize], [hex usize]) {
            segments.push(Segment {
                start_address: a,
                size: b - a,
            })
        } else {
            println!("Unable to parse maps line: {}", line);
        }
    }
    Ok(segments)
}

fn set_idlemap() -> std::io::Result<()> {
    let idle_bitmap_data: Vec<u8> = vec![0xff; 4096];
    let mut file = File::create(IDLE_BITMAP_PATH)?;
    let mut write_counter: usize = 0;
    while {
        let wrote = file.write(&idle_bitmap_data)?;
        write_counter += wrote;
        wrote == idle_bitmap_data.len()
    } {}
    if write_counter * 8 < system_ram_pages() {
        println!("Fatal: unable to set sufficient idlemap pages. Only set {} bytes", write_counter);
        std::process::exit(1);
    }
    Ok(())
}

fn load_idlemap() -> std::io::Result<Vec<u8>> {
    // kinda weird, but what we're doing here is 8-bits per page, but actually dividing by 7
    // to have the capacity a bit bigger, seeing as we seem to fill about 1.1 times the number
    // of idlemap bits as we have physical pages.
    let mut idlemap: Vec<u8> = Vec::with_capacity(system_ram_pages() / 7);
    println!("Set vector to have capacity: {} and size {}", idlemap.capacity(), idlemap.len());
    let mut file = File::open(IDLE_BITMAP_PATH)?;
    file.read_to_end(&mut idlemap)?;
    println!("After read have capacity: {} and size {}", idlemap.capacity(), idlemap.len());
    Ok(idlemap)
}

// this is fucking inefficient; we should be opening the file ones and
// reading contiguous pages in one go.
// but it'll do for a start....
fn get_page_content(pid: i32, page_addr_start: usize, pages: usize) -> std::io::Result<Vec<u8>> {
    let mut mem_file = File::open(format!("/proc/{}/mem", pid))?;
    mem_file.seek(SeekFrom::Start(page_addr_start as u64))?;
    let mut mem: Vec<u8> = Vec::with_capacity(pages * PAGE_SIZE);
    mem.resize(pages * PAGE_SIZE, 0); // why do I have to do this...?
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
