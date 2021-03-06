//extern crate data_encoding;
extern crate ring;
extern crate mem_analyze;
extern crate simplelog;
extern crate clap;
extern crate telnet;

#[macro_use]
extern crate log;

use std::env;
use simplelog::*;
use chrono::Utc;
use clap::{Arg, App};

const SLEEP_TIME: u64 = 10;

fn main() -> std::io::Result<()> {

    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Debug, Config::default()).unwrap(),
        ]
    ).unwrap();

    let matches = App::new("MemAnalyze")
        .version("0.1")
        .author("jgowans")
        .arg(Arg::with_name("region")
             .short("r")
             .long("region")
             .takes_value(true))
        .arg(Arg::with_name("pid")
             .short("p")
             .long("pid")
             .takes_value(true)
             .multiple(true))
        .arg(Arg::with_name("sleep")
             .short("s")
             .long("sleep")
             .takes_value(true))
        .arg(Arg::with_name("pageout")
             .long("pageout")
             .takes_value(true))
        .arg(Arg::with_name("inspect-ram")
             .short("i")
             .long("inspect-ram")
             .multiple(true))
        .arg(Arg::with_name("s3-persist")
             .long("s3")
             .multiple(true))
        .get_matches();

    let region: String = match matches.value_of("region") {
        Some(region) => region.to_string(),
        None => match env::var("EC2_PUBLIC_REGION") {
            Ok(region) => region.to_string(),
            Err(_e) => panic!("Region not passed not available from env var")
        }
    };

    let pids: Vec<i32> = match matches.values_of("pid") {
        Some(values) => values.map(|p| p.parse().expect("Can't parse to i32")).collect(),
        None => Vec::new(),
    };

    let sleep: u64 = match matches.value_of("sleep") {
        Some(time) => time.parse().expect("time must be u64"),
        None => SLEEP_TIME,
    };

    let pageout: u64 = match matches.value_of("pageout") {
        Some(pageout) => pageout.parse().expect("pageout must be u64"),
        None => 0,
    };

    let inspect_ram: bool = matches.is_present("inspect-ram");
    let s3_persist: bool = matches.is_present("s3-persist");

    let mut vmm = mem_analyze::vmm::Vmm::new();

    if pids.len() > 0 {
        info!("PID supplied: {:?}\n", pids);
        loop {
            let start_time = Utc::now();
            let process_memory = mem_analyze::dump::get_memory(pids[0], sleep)?;
            mem_analyze::statistics::page_analytics(pids[0], &process_memory);
            mem_analyze::persist::write_process_memory(pids[0], &region, &process_memory, s3_persist)?;
            vmm.swap_some_out(&process_memory.segments[0], pageout);
            info!("---------- Completed analysis in in {} ms ----------",
                  (Utc::now() - start_time).num_milliseconds());
        }
    } else {
        info!("No PIDs; analyzing whole system\n");
        loop {
            let start_time = Utc::now();
            let process_memory = mem_analyze::dump::get_host_memory(sleep, inspect_ram)?;
            mem_analyze::statistics::page_analytics(pids[0], &process_memory);
            mem_analyze::persist::write_process_memory(0, &region, &process_memory, s3_persist)?;
            info!("---------- Completed analysis in in {} ms ----------",
                  (Utc::now() - start_time).num_milliseconds());
        }
    }
}
