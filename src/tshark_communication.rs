use crate::http::tshark_http;
use crate::http2::tshark_http2;
use crate::pgsql::tshark_pgsql;
use chrono::NaiveDateTime;
use quick_xml::events::Event;
use std::io::BufReader;
use std::process::ChildStdout;
use std::str;

#[derive(Debug)]
pub struct TSharkPacket {
    pub frame_time: NaiveDateTime,
    pub ip_src: String, // v4 or v6
    pub ip_dst: String, // v4 or v6
    pub tcp_seq_number: u32,
    pub tcp_stream_id: u32,
    pub port_src: u32,
    pub port_dst: u32,
}

pub fn parse_packet(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> Result<TSharkPacket, quick_xml::Error> {
    let mut frame_time;
    let mut ip_src;
    let mut ip_dst;
    let mut tcp_seq_number;
    let mut tcp_stream_id;
    let mut port_src;
    let mut port_dst;
    loop {
        match xml_reader.read_event(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if e.name() == b"proto" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"frame") => {
                            frame_time = parse_frame_info(xml_reader, buf);
                        }
                        Some(b"ip") => {
                            let ip_info = parse_ip_info(xml_reader, buf);
                            ip_src = ip_info.0;
                            ip_dst = ip_info.1;
                        }
                        // TODO ipv6
                        Some(b"tcp") => {
                            // waiting for https://github.com/rust-lang/rust/issues/71126
                            let tcp_info = parse_tcp_info(xml_reader, buf);
                            tcp_seq_number = tcp_info.0;
                            tcp_stream_id = tcp_info.1;
                            port_src = tcp_info.2;
                            port_dst = tcp_info.3;
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"packet" {
                    return Ok(TSharkPacket {
                        frame_time,
                        ip_src,
                        ip_dst,
                        tcp_seq_number,
                        tcp_stream_id,
                        port_src,
                        port_dst,
                    });
                }
            }
            Err(e) => return Err(e),
            _ => {}
        }
        // buf.clear();
    }
}

fn parse_frame_info(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> NaiveDateTime {
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field"
                    && e.attributes().any(|kv| {
                        kv.unwrap() == ("name".as_bytes(), "frame.time".as_bytes()).into()
                    })
                {
                    // dbg!(e);
                    // panic!();
                    if let Some(time_str) = e.attributes().find_map(|a| {
                        Some(a.unwrap())
                            .filter(|a| a.key == b"show")
                            .map(|a| String::from_utf8(a.value.to_vec()).unwrap())
                    }) {
                        // must use NaiveDateTime because chrono can't read string timezone names.
                        // https://docs.rs/chrono/0.4.19/chrono/format/strftime/index.html#specifiers
                        // > %Z: Offset will not be populated from the parsed data, nor will it be validated.
                        // > Timezone is completely ignored. Similar to the glibc strptime treatment of this format code.
                        // > It is not possible to reliably convert from an abbreviation to an offset, for example CDT
                        // > can mean either Central Daylight Time (North America) or China Daylight Time.
                        return NaiveDateTime::parse_from_str(&time_str, "%b %e, %Y %T.%f %Z")
                            .unwrap();
                    }
                }
            }
        }
    }
}

fn parse_ip_info(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> (String, String) {
    let mut ip_src;
    let mut ip_dst;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"ip.src") => {
                            ip_src =
                                String::from_utf8(element_attr_val(e, b"show").to_vec()).unwrap();
                        }
                        Some(b"ip.dst") => {
                            ip_dst =
                                String::from_utf8(element_attr_val(e, b"show").to_vec()).unwrap();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return (ip_src, ip_dst);
                }
            }
        }
    }
}

fn parse_tcp_info(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> (u32, u32, u32, u32) {
    let mut tcp_seq_number;
    let mut tcp_stream_id;
    let mut port_src;
    let mut port_dst;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"tcp.srcport") => {
                            port_src = str::from_utf8(element_attr_val(e, b"show"))
                                .unwrap()
                                .parse()
                                .unwrap();
                        }
                        Some(b"tcp.dstport") => {
                            port_dst = str::from_utf8(element_attr_val(e, b"show"))
                                .unwrap()
                                .parse()
                                .unwrap();
                        }
                        Some(b"tcp.seq_raw") => {
                            port_dst = str::from_utf8(element_attr_val(e, b"show"))
                                .unwrap()
                                .parse()
                                .unwrap();
                        }
                        Some(b"tcp.stream") => {
                            tcp_stream_id = str::from_utf8(element_attr_val(e, b"show"))
                                .unwrap()
                                .parse()
                                .unwrap();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return (tcp_seq_number, tcp_stream_id, port_src, port_dst);
                }
            }
        }
    }
}

fn element_attr_val<'a>(
    e: &'a quick_xml::events::BytesStart<'a>,
    attr_name: &'static [u8],
) -> &'a [u8] {
    &*e.attributes()
        .find(|kv| &*kv.unwrap().key == attr_name)
        .unwrap()
        .unwrap()
        .value
}
