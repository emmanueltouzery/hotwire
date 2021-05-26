use crate::http::tshark_http;
use crate::http2::tshark_http2;
use crate::pgsql::tshark_pgsql;
use crate::widgets::win;
use chrono::NaiveDateTime;
use quick_xml::events::Event;
use std::fmt::Debug;
use std::io::BufRead;
use std::str;
use std::str::FromStr;

macro_rules! xml_event_loop {
    ($reader:ident, $buf:ident, $($tts:tt)*) => {
        loop {
            match $reader.read_event($buf) {
                $($tts)*
                Ok(Event::Eof) => panic!("Unexpected EOF"),
                Ok(_) => {}
                Err(e) => panic!("Error at position {}: {:?}", $reader.buffer_position(), e),
            }
        }
    }
}

// macro_rules! xml_event_loop_debug {
//     ($reader:ident, $buf:ident, $($tts:tt)*) => {
//         let p = $reader.buffer_position();
//         // println!("Start loop at position {}", p);
//         loop {
//             let evt = $reader.read_event($buf);
//             // if p >= 945409200 {
//             if !matches!(evt, Result::Ok(Event::Text(_))) {
//                 dbg!(&evt);
//             }
//             // }
//             match evt {
//                 $($tts)*
//                 Ok(Event::Eof) => panic!("Unexpected EOF"),
//                 Ok(_) => {}
//                 Err(e) => panic!("Error at position {}: {:?}", $reader.buffer_position(), e),
//             }
//         }
//     }
// }

#[derive(Debug)]
pub struct TSharkPacketBasicInfo {
    pub frame_time: NaiveDateTime,
    pub ip_src: String, // v4 or v6
    pub ip_dst: String, // v4 or v6
    pub tcp_seq_number: u32,
    pub tcp_stream_id: u32,
    pub port_src: u32,
    pub port_dst: u32,
}

#[derive(Debug)]
pub struct TSharkPacket {
    pub basic_info: TSharkPacketBasicInfo,
    pub http: Option<tshark_http::TSharkHttp>,
    pub http2: Option<Vec<tshark_http2::TSharkHttp2Message>>,
    pub pgsql: Option<Vec<tshark_pgsql::PostgresWireMessage>>,
}

pub fn parse_packet<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<TSharkPacket, quick_xml::Error> {
    let mut frame_time = NaiveDateTime::from_timestamp(0, 0);
    let mut ip_src = None;
    let mut ip_dst = None;
    let mut tcp_seq_number = 0;
    let mut tcp_stream_id = 0;
    let mut port_src = 0;
    let mut port_dst = 0;
    let mut http = None;
    let mut http2 = None::<Vec<tshark_http2::TSharkHttp2Message>>;
    let mut pgsql = None::<Vec<tshark_pgsql::PostgresWireMessage>>;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Start(ref e)) => {
            if e.name() == b"proto" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"frame") => {
                        frame_time = parse_frame_info(xml_reader);
                    }
                    Some(b"ip") => {
                        if ip_src.is_some() {
                            panic!("Unexpected IP at position {}", xml_reader.buffer_position());
                        }
                        let ip_info = parse_ip_info(xml_reader);
                        ip_src = Some(ip_info.0);
                        ip_dst = Some(ip_info.1);
                    }
                    // TODO ipv6
                    Some(b"tcp") => {
                        // waiting for https://github.com/rust-lang/rust/issues/71126
                        let tcp_info = parse_tcp_info(xml_reader);
                        tcp_seq_number = tcp_info.0;
                        tcp_stream_id = tcp_info.1;
                        port_src = tcp_info.2;
                        port_dst = tcp_info.3;
                    }
                    Some(b"http") => {
                        if http.is_some() {
                            panic!("http already there");
                        }
                        http = Some(tshark_http::parse_http_info(xml_reader));
                    }
                    Some(b"http2") => {
                        let mut http2_packets = tshark_http2::parse_http2_info(xml_reader);
                        if let Some(mut sofar) = http2 {
                            sofar.append(&mut http2_packets);
                            http2 = Some(sofar);
                        } else {
                            http2 = Some(http2_packets);
                        }
                    }
                    Some(b"pgsql") => {
                        if let Some(pgsql_packets) = tshark_pgsql::parse_pgsql_info(xml_reader) {
                            if let Some(mut sofar) = pgsql {
                                sofar.push(pgsql_packets);
                                pgsql = Some(sofar);
                            } else {
                                pgsql = Some(vec![pgsql_packets]);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"packet" {
                return Ok(TSharkPacket {
                    basic_info: TSharkPacketBasicInfo {
                        frame_time,
                        ip_src: ip_src.unwrap_or_default(),
                        ip_dst: ip_dst.unwrap_or_default(),
                        tcp_seq_number,
                        tcp_stream_id,
                        port_src,
                        port_dst,
                    },
                    http,
                    http2,
                    pgsql,
                });
            }
        }
    )
}

fn parse_frame_info<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> NaiveDateTime {
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
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
    )
}

fn parse_ip_info<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> (String, String) {
    let mut ip_src = None;
    let mut ip_dst = None;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"ip.src") => {
                        ip_src = element_attr_val_string(e, b"show");
                    }
                    Some(b"ip.dst") => {
                        ip_dst = element_attr_val_string(e, b"show");
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return (ip_src.unwrap_or_default(), ip_dst.unwrap_or_default());
            }
        }
    )
}

fn parse_tcp_info<B: BufRead>(xml_reader: &mut quick_xml::Reader<B>) -> (u32, u32, u32, u32) {
    let mut tcp_seq_number = 0;
    let mut tcp_stream_id = 0;
    let mut port_src = 0;
    let mut port_dst = 0;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = e
                    .attributes()
                    .find(|kv| kv.as_ref().unwrap().key == "name".as_bytes())
                    .map(|kv| kv.unwrap().value);
                match name.as_deref() {
                    Some(b"tcp.srcport") => {
                        port_src = element_attr_val_number(e, b"show").unwrap();
                    }
                    Some(b"tcp.dstport") => {
                        port_dst = element_attr_val_number(e, b"show").unwrap();
                    }
                    Some(b"tcp.seq_raw") => {
                        tcp_seq_number = element_attr_val_number(e, b"show").unwrap();
                    }
                    Some(b"tcp.stream") => {
                        tcp_stream_id = element_attr_val_number(e, b"show").unwrap();
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return (tcp_seq_number, tcp_stream_id, port_src, port_dst);
            }
        }
    )
}

pub fn element_attr_val_number<'a, F: FromStr>(
    e: &'a quick_xml::events::BytesStart<'a>,
    attr_name: &'static [u8],
) -> Option<F>
where
    <F as FromStr>::Err: Debug,
{
    str::from_utf8(
        e.attributes()
            .find(|kv| kv.as_ref().unwrap().key == attr_name)
            .unwrap()
            .unwrap()
            .unescaped_value()
            .unwrap()
            .as_ref(),
    )
    .unwrap()
    .parse()
    .ok()
}

pub fn element_attr_val_string<'a>(
    e: &'a quick_xml::events::BytesStart<'a>,
    attr_name: &'static [u8],
) -> Option<String> {
    e.attributes()
        .find(|kv| kv.as_ref().unwrap().key == attr_name)
        .and_then(|v| v.ok())
        .and_then(|v| {
            let st = v.unescaped_value().ok();
            st.map(|v| v.to_vec())
        })
        .and_then(|v| String::from_utf8(v).ok())
}

#[cfg(test)]
macro_rules! test_fmt_str {
    () => {
        r#"
   <pdml>
     <packet>
       <proto name="frame">
           <field name="frame.time" show="Mar  5, 2021 08:49:52.736275000 CET"/>
       </proto>
       <proto name="ip">
           <field name="ip.src" show="10.215.215.9" />
           <field name="ip.dst" show="10.215.215.9" />
       </proto>
       <proto name="tcp">
           <field name="tcp.srcport" show="52796" value="ce3c"/>
           <field name="tcp.dstport" show="5432" value="1538"/>
           <field name="tcp.seq_raw" show="1963007432" value="75011dc8"/>
           <field name="tcp.stream" show="4"/>
       </proto>
       {}
     </packet>
   </pdml>
"#
    };
}

#[cfg(test)]
pub fn parse_test_xml(xml: &str) -> Vec<TSharkPacket> {
    win::parse_pdml_stream(format!(test_fmt_str!(), xml).as_bytes()).unwrap()
}
