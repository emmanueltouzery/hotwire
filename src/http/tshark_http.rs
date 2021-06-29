use crate::tshark_communication;
use quick_xml::events::Event;
use std::io::BufRead;

#[derive(Debug, Copy, Clone)]
pub enum HttpType {
    Request,
    Response,
}

#[derive(Debug)]
pub struct TSharkHttp {
    // 99.99% of the time we can infer the HttpType, and so we wouldn't need Option.
    // I have seen a weird response from httpstat.us though...
    // The response was...
    //   <proto name="http" showname="Hypertext Transfer Protocol" size="5" pos="60">
    //     <field name="http.file_data" showname="File Data: 5 bytes" size="5" pos="60" show="0^M^M" value="300d0a0d0a"/>
    //     <field name="data" value="300d0a0d0a">
    //       <field name="data.data" showname="Data: 300d0a0d0a" size="5" pos="60" show="30:0d:0a:0d:0a" value="300d0a0d0a"/>
    //       <field name="data.len" showname="Length: 5" size="0" pos="60" show="5"/>
    //     </field>
    //   </proto>
    //   In other words the response contained only "0\r\n\r\n", without response code or anything.
    //   Weird as it is, I'd like to make a best effort to support it.
    pub http_type: Option<HttpType>,
    pub http_host: Option<String>,
    pub first_line: String,
    pub other_lines: String,
    pub body: Option<Vec<u8>>,
    pub content_type: Option<String>,
}

pub fn parse_http_info<B: BufRead>(
    xml_reader: &mut quick_xml::Reader<B>,
) -> Result<TSharkHttp, String> {
    let mut http_type = None;
    let mut http_host = None;
    let mut first_line = None;
    let mut other_lines = vec![];
    let mut body = None;
    let mut content_type = None;
    let buf = &mut vec![];
    xml_event_loop!(xml_reader, buf,
        Ok(Event::Start(ref e)) => {
            if e.name() == b"field" {
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                if name.as_deref() == Some(b"") && first_line.is_none() {
                    first_line = tshark_communication::element_attr_val_string(e, b"show")?
                        .map(|t| t.trim_end_matches("\\r\\n").to_string())
                }
            }
        }
        Ok(Event::Empty(ref e)) => {
            if e.name() == b"field" {
                let name = tshark_communication::attr_by_name(&mut e.attributes(), b"name")?;
                match name.as_deref() {
                    Some(b"http.content_type") => {
                        content_type = tshark_communication::element_attr_val_string(e, b"show")?
                    }
                    Some(b"http.host") => {
                        http_host = tshark_communication::element_attr_val_string(e, b"show")?
                    }
                    Some(b"http.request.line") => {
                        http_type = Some(HttpType::Request);
                        other_lines.push(
                            tshark_communication::element_attr_val_string(e, b"show")?.unwrap(),
                        );
                    }
                    Some(b"http.response.line") => {
                        http_type = Some(HttpType::Response);
                        other_lines.push(
                            tshark_communication::element_attr_val_string(e, b"show")?.unwrap(),
                        );
                    }
                    Some(b"http.file_data") => {
                        // binary will be in "value", text in "show"
                        let hex_body = if let Some(v) = tshark_communication::element_attr_val_string(e, b"value")? {
                            Some(v)
                        } else {
                            tshark_communication::element_attr_val_string(e, b"show")?
                        };
                        if let Some(b) = hex_body {
                            body = Some(hex::decode(b.replace(':', ""))
                                        .map_err(|e| format!("Invalid hex string: {} {:?}", b, e))?);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Event::End(ref e)) => {
            if e.name() == b"proto" {
                return Ok(TSharkHttp {
                    http_type,
                    http_host,
                    first_line: first_line.unwrap_or_default(),
                    other_lines: other_lines.join(""),
                    body,
                    content_type,
                });
            }
        }
    )
}
