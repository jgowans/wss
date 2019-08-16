use telnet::Telnet;
use telnet::TelnetEvent;
use rand::thread_rng;
use rand::distributions::{Distribution, Uniform};

pub struct Vmm {
    telnet: Telnet,
}

impl Vmm {
    pub fn new() -> Vmm {
        let mut vmm = match Telnet::connect(("127.0.0.1", 4444), 256) {
            Ok(telnet) => Vmm { telnet: telnet },
            Err(e) => panic!("Unable to establish telnet session {:?}", e),
        };
        match vmm.telnet.read().unwrap() {
            TelnetEvent::Data(d) => print!("{}", std::str::from_utf8(&d).unwrap()),
            _ => print!("Other?"),
        }
        vmm.telnet.write(b"{ \"execute\": \"qmp_capabilities\" }").unwrap();
        match vmm.telnet.read().unwrap() {
            TelnetEvent::Data(d) => print!("{}", std::str::from_utf8(&d).unwrap()),
            _ => print!("Other?"),
        }
        vmm
    }

    pub fn swap_some_out(&mut self, segment: &super::Segment, pages_to_swap: u64) {
        info!("Selecting pages to sample...");
        let idle_pages: Vec<usize> = segment.page_flags.iter().enumerate()
            .filter(|(_idx, &val)| val & (1 << super::PRESENT_PAGE_BIT) != 0)
            .filter(|(_idx, &val)| val & (1 << super::ACTIVE_PAGE_BIT) == 0)
            .map(|(idx, _val)| idx)
            .collect();
        // there's gotta be a nicer way to select a few random elements.... 
        let mut rng = rand::thread_rng();
        let between = Uniform::new(0, idle_pages.len());
        let selected: Vec<usize> = (0..pages_to_swap).map(|_| idle_pages[between.sample(&mut rng)]).collect();
        let data = object!{
            "execute" => "pageout",
            "arguments" => object!{"pages" => selected }
        };
        //info!("Would send {:?}", data.dump());
        self.telnet.write(data.dump().as_bytes()).unwrap();
        match self.telnet.read().unwrap() {
            TelnetEvent::Data(d) => print!("{}", std::str::from_utf8(&d).unwrap()),
            _ => print!("Other?"),
        }
    }
}
