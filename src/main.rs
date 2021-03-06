use crate::tshark_packet::TSharkPacket;
use itertools::Itertools;
use serde_json::Value;
use std::process::Command;

mod tshark_packet;

fn main() {
    println!("hello");
    let tshark_output = Command::new("tshark")
        .args(&["-r", "/home/emmanuel/dump_afc.pcap", "-Tjson"])
        .output()
        .expect("failed calling tshark");
    if !tshark_output.status.success() {
        eprintln!("tshark returned error code {}", tshark_output.status);
        std::process::exit(1);
    }
    let output_str =
        std::str::from_utf8(&tshark_output.stdout).expect("tshark output is not valid utf8");
    match serde_json::from_str::<Value>(output_str) {
        Ok(Value::Array(vals)) => {
            let packets = vals
                .into_iter()
                .map(TSharkPacket::new)
                .collect::<Option<Vec<_>>>()
                .expect("unexpected tshark json format");
            handle_packets(&packets)
        }
        _ => panic!("tshark output is not valid json"),
    }
}

fn handle_packets(packets: &[TSharkPacket]) {
    let by_stream = packets
        .into_iter()
        .map(|p| (p.get_tcp_stream(), p))
        .into_group_map();
    println!("{} streams", by_stream.len());
    // for packet in packets {
    //     println!("{:?}", packet.get_ip_src());
    // }
}
