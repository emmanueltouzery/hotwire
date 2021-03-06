use crate::tshark_packet::TSharkPacket;
use itertools::Itertools;
use std::fs::File;
use std::io::Read;
use std::process::Command;

mod tshark_packet;

fn main() {
    println!("hello");
    // let tshark_output = Command::new("tshark")
    //     .args(&["-r", "/home/emmanuel/dump_afc.pcap", "-Tjson"])
    //     .output()
    //     .expect("failed calling tshark");
    // if !tshark_output.status.success() {
    //     eprintln!("tshark returned error code {}", tshark_output.status);
    //     std::process::exit(1);
    // }
    // let output_str =
    //     std::str::from_utf8(&tshark_output.stdout).expect("tshark output is not valid utf8");
    let mut f = File::open("parsed.json").unwrap();
    let mut output_str = String::new();
    f.read_to_string(&mut output_str).unwrap();
    match serde_json::from_str::<Vec<TSharkPacket>>(&output_str) {
        Ok(packets) => handle_packets(&packets),
        Err(e) => panic!(format!("tshark output is not valid json: {:?}", e)),
    }
}

fn handle_packets(packets: &[TSharkPacket]) {
    let by_stream = packets
        .into_iter()
        .map(|p| (p.source.layers.tcp.as_ref().map(|t| &t.stream), p))
        .into_group_map();
    println!("{} streams", by_stream.len());
    // for packet in packets {
    //     println!("{:?}", packet.get_ip_src());
    // }
}
