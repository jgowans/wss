pub fn page_analytics(memory: &super::ProcessMemory) {
    let mut total_pages = 0;
    let mut zero_pages = 0;
    let mut unmapped_pages = 0;
    let mut swapped_pages = 0;
    let mut idle_pages = 0;
    let mut active_pages = 0;
    let mut repeating_64_bit_patterns = 0;
    for segment in &memory.segments {
        for page in &segment.pages {
            total_pages += 1;
            match page.status {
                super::PageStatus::Unmapped => unmapped_pages += 1,
                super::PageStatus::Swapped => swapped_pages += 1,
                super::PageStatus::Idle => idle_pages += 1,
                super::PageStatus::Active => active_pages += 1,
            }
            if page.is_zero() {
                zero_pages += 1;
            }
            if page.repeating_64_bit_pattern() {
                repeating_64_bit_patterns += 1;
            }
        }
        debug!("Segment start {:x} with size {}", segment.virtual_addr_start, segment.pages.len());
    }
    info!("Total pages: {}", total_pages);
    info!("Unmapped pages: {}", unmapped_pages);
    info!("Zero pages: {}", zero_pages);
    info!("Active pages: {}", active_pages);
    info!("Idle pages: {}", idle_pages);
    info!("Swapped pages: {}", swapped_pages);
    info!("Repeating 64-bit patterns : {}", repeating_64_bit_patterns);
}
