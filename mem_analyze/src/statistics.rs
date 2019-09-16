use std::fs::{File, OpenOptions};
use std::io::Read;
use csv::Writer;
use sysinfo::{System, SystemExt, ProcessExt, RefreshKind};

pub fn page_analytics(pid: i32, memory: &super::ProcessMemory) {
    let mut total_pages = 0;
    let mut lru_pages = 0;
    let mut zero_pages = 0;
    let mut active_pages = 0;
    let mut present_pages = 0;
    for segment in &memory.segments {
        for page_flags in &segment.page_flags {
            total_pages += 1;
            if page_flags & (1 << super::LRU_PAGE_BIT) != 0 {
                lru_pages += 1;
            }
            if page_flags & (1 << super::ZERO_PAGE_BIT) != 0 {
                zero_pages += 1;
            }
            if page_flags & (1 << super::ACTIVE_PAGE_BIT) != 0 {
                active_pages += 1;
            }
            if page_flags & (1 << super::PRESENT_PAGE_BIT) != 0 {
                present_pages += 1;
            }
        }
        debug!("Segment start {:x} with size {}", segment.addr_start, segment.page_flags.len());
    }
    info!("Total pages: {}", total_pages);
    log_info("LRU", lru_pages, total_pages);
    log_info("Zero", zero_pages, total_pages);
    log_info("Active", active_pages, total_pages);
    log_info("Present", present_pages, total_pages);

    let mut row: Vec<String> = vec![
        memory.timestamp.timestamp().to_string(),
        total_pages.to_string(),
        lru_pages.to_string(),
        zero_pages.to_string(),
        active_pages.to_string(),
        present_pages.to_string()];
    append_process_stats(pid, memory, &mut row);
    let mut wtr = Writer::from_writer(
        OpenOptions::new().append(true).create(true)
            .open(format!("/tmp/wss/{}.csv", pid)).unwrap());
    wtr.write_record(row).unwrap();
    wtr.flush().unwrap();

    fn log_info(name: &str, val: i64, total: i64) {
        info!("{}", format!("{} pages: {} = {:.1}%", name, val, 100.0 * val as f32 / total as f32));
    }
}

fn append_process_stats(pid: i32, memory: &super::ProcessMemory, row: &mut Vec<String>) {
    // TODO: error handling. :P
    let mut system = System::new_with_specifics(RefreshKind::new());
    system.refresh_process(pid);
    let process = system.get_process(pid).unwrap();
    row.push(process.minflt().to_string());
    row.push(process.majflt().to_string());

    let mut smaps = String::new();
    File::open(format!("/proc/{}/smaps", pid)).unwrap().read_to_string(&mut smaps).unwrap();
    let swap_usage: u64 = memory.segments.iter()
        .map(|segment| swap_for_segment(&smaps, segment))
        .fold(0,|a, b| a + b);
    info!("Swap usage: {} kB", swap_usage >> 10);
    row.push(swap_usage.to_string());
}

fn swap_for_segment(smaps: &str, segment: &super::Segment) -> u64 {
    let target_line = format!("{:x}", segment.addr_start);
    let mut lines = smaps.lines();
    loop {
        match lines.next() {
            Some(line) => {
                if line.starts_with(&target_line) {
                    loop {
                        match lines.next() {
                            Some(line) => {
                                // Swap:            3237444 kB
                                if line.starts_with("Swap:") {
                                    return u64::from_str_radix(line.split_ascii_whitespace().nth(1).unwrap(), 10).unwrap() << 10
                                }
                            },
                            None => panic!("Got to end of smaps block withouht finding swap")
                        }
                    }
                }
            },
            None => panic!("Got to end of smaps file without finding {}", target_line)
        }
    }
}
