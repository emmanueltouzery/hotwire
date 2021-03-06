use crate::tshark_communication::TSharkCommunication;
use itertools::Itertools;
use relm::Widget;
use std::cmp::Reverse;
use std::fs::File;
use std::io::Read;
use std::process::Command;

mod tshark_communication;
mod widgets;

fn main() {
    println!("hello");
    let tshark_output = Command::new("tshark")
        .args(&["-r", "/home/emmanuel/dump_afc.pcap", "-Tjson", "http"])
        .output()
        .expect("failed calling tshark");
    if !tshark_output.status.success() {
        eprintln!("tshark returned error code {}", tshark_output.status);
        std::process::exit(1);
    }
    let output_str =
        std::str::from_utf8(&tshark_output.stdout).expect("tshark output is not valid utf8");
    // let mut f = File::open("parsed.json").unwrap();
    // let mut output_str = String::new();
    // f.read_to_string(&mut output_str).unwrap();
    match serde_json::from_str::<Vec<TSharkCommunication>>(&output_str) {
        Ok(packets) => handle_packets(packets),
        Err(e) => panic!(format!("tshark output is not valid json: {:?}", e)),
    }
}

fn handle_packets(packets: Vec<TSharkCommunication>) {
    let mut by_stream: Vec<_> = packets
        .into_iter()
        // .filter(|p| p.source.layers.http.is_some())
        .map(|p| (p.source.layers.tcp.as_ref().map(|t| t.stream), p))
        .into_group_map()
        .into_iter()
        .collect();
    by_stream.sort_by_key(|p| Reverse(p.1.len()));
    println!(
        "{} streams, length as from {:?} to {:?}.",
        by_stream.len(),
        by_stream.first().map(|f| f.1.len()),
        by_stream.last().map(|l| l.1.len())
    );
    println!("src desc count");
    for stream in &by_stream[0..10] {
        let layers = &stream.1.first().as_ref().unwrap().source.layers;
        let ip = layers.ip.as_ref().unwrap();
        let tcp = layers.tcp.as_ref().unwrap();
        println!(
            "{}:{} -> {}:{} {}",
            ip.ip_src,
            tcp.port_src,
            ip.ip_dst,
            tcp.port_dst,
            stream.1.len()
        );
    }
    // for packet in &by_stream.first().unwrap().1 {
    //     println!("{:?}", packet.source.layers.http);
    // }
    widgets::win::Win::run(by_stream).unwrap();
}
