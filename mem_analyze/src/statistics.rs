use std::fs::OpenOptions;
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
    append_process_stats(pid, &mut row);
    let mut wtr = Writer::from_writer(
        OpenOptions::new().append(true).create(true)
            .open(format!("/tmp/wss/{}.csv", pid)).unwrap());
    wtr.write_record(row).unwrap();
    wtr.flush().unwrap();

    fn log_info(name: &str, val: i64, total: i64) {
        info!("{}", format!("{} pages: {} = {:.1}%", name, val, 100.0 * val as f32 / total as f32));
    }
}

fn append_process_stats(pid: i32, row: &mut Vec<String>) {
    // TODO: error handling. :P
    let mut system = System::new_with_specifics(RefreshKind::new());
    system.refresh_process(pid);
    let process = system.get_process(pid).unwrap();
    row.push(process.minflt().to_string());
    row.push(process.majflt().to_string());
}
