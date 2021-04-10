use super::comm_info_header;
use super::comm_info_header::CommInfoHeader;
use super::http_message_parser::HttpMessageData;
use super::message_parser::MessageInfo;
use crate::tshark_communication_raw::TSharkCommunicationRaw;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use gdk_pixbuf::prelude::*;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;

const TEXT_CONTENTS_STACK_NAME: &str = "text";
const IMAGE_CONTENTS_STACK_NAME: &str = "image";

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageInfo),
    GotImage(Vec<u8>),
}

pub struct Model {
    stream_id: u32,
    client_ip: String,
    data: HttpMessageData,

    _got_image_channel: relm::Channel<Vec<u8>>,
    got_image_sender: relm::Sender<Vec<u8>>,
}

#[widget]
impl Widget for HttpCommEntry {
    fn model(relm: &relm::Relm<Self>, params: (u32, String, HttpMessageData)) -> Model {
        let (stream_id, client_ip, data) = params;
        let stream = relm.stream().clone();
        let (_got_image_channel, got_image_sender) =
            relm::Channel::new(move |r: Vec<u8>| stream.emit(Msg::GotImage(r)));
        Model {
            data,
            stream_id,
            client_ip,
            _got_image_channel,
            got_image_sender,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::DisplayDetails(
                bg_sender,
                file_path,
                MessageInfo {
                    client_ip,
                    stream_id,
                    message_data: MessageData::Http(msg),
                },
            ) => {
                match (
                    &msg.response.as_ref().and_then(|r| r.content_type.as_ref()),
                    self.model
                        .data
                        .response
                        .as_ref()
                        .and_then(|r| r.body.as_ref()),
                ) {
                    (Some(content_type), Some(body))
                        if content_type.starts_with("image/") && msg.response.is_some() =>
                    {
                        let seq_no = msg.response.as_ref().unwrap().tcp_seq_number;
                        let s = self.model.got_image_sender.clone();
                        bg_sender
                            .send(BgFunc::new(move || {
                                Self::load_image(&file_path, seq_no, s.clone())
                            }))
                            .unwrap();
                    }
                    _ => {
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(TEXT_CONTENTS_STACK_NAME);
                    }
                }
                self.model.data = msg;
                self.streams
                    .comm_info_header
                    .emit(comm_info_header::Msg::Update(client_ip.clone(), stream_id));
                self.model.stream_id = stream_id;
                self.model.client_ip = client_ip;
            }
            Msg::GotImage(bytes) => {
                let loader = gdk_pixbuf::PixbufLoader::new();
                loader.write(&bytes).unwrap();
                loader.close().unwrap();
                self.widgets
                    .body_image
                    .set_from_pixbuf(loader.get_pixbuf().as_ref());
                self.widgets
                    .contents_stack
                    .set_visible_child_name(IMAGE_CONTENTS_STACK_NAME);
            }
            _ => {}
        }
    }

    fn load_image(file_path: &Path, tcp_seq_number: u32, s: relm::Sender<Vec<u8>>) {
        let mut packets = win::invoke_tshark::<TSharkCommunicationRaw>(
            file_path,
            win::TSharkMode::JsonRaw,
            &format!("tcp.seq eq {}", tcp_seq_number),
        )
        .expect("tshark error");
        if packets.len() == 1 {
            let bytes = packets.pop().unwrap().source.layers.http.unwrap().file_data;
            s.send(bytes).unwrap();
        } else {
            panic!(format!(
                "unexpected json from tshark, tcp stream {}",
                tcp_seq_number
            ));
        }
    }

    fn highlight_indent<'a>(body: &str, content_type: Option<&str>) -> String {
        // support eg "application/xml;charset=UTF8"
        let content_type_first_part = content_type.map(|c| {
            if let Some(semicolon_index) = c.find(';') {
                &c[0..semicolon_index]
            } else {
                c
            }
        });
        match content_type_first_part {
            Some("application/xml") | Some("text/xml") => Self::highlight_indent_xml(body),
            _ => glib::markup_escape_text(body).to_string(),
        }
    }

    fn highlight_indent_xml(xml: &str) -> String {
        let mut indent = 0;
        let mut result = "".to_string();
        let mut has_attributes = false;
        let mut has_text = false;
        let mut attrs_on_line = 0;
        for token in xmlparser::Tokenizer::from(xml) {
            dbg!(indent);
            dbg!(token);
            match token {
                Ok(xmlparser::Token::ElementStart { local, .. }) => {
                    if result.len() > 0 {
                        result.push_str("\n");
                        for _ in 0..indent {
                            result.push_str("  ");
                        }
                    }
                    result.push_str("&lt;<b>");
                    result.push_str(&local);
                    has_attributes = false;
                    has_text = false;
                    attrs_on_line = 0;
                }
                Ok(xmlparser::Token::Attribute { span, .. }) => {
                    if !has_attributes {
                        result.push_str("</b>");
                    }
                    attrs_on_line += 1;
                    if attrs_on_line > 3 {
                        result.push_str("\n");
                        for _ in 0..(indent + 1) {
                            result.push_str("  ");
                        }
                        attrs_on_line = 0;
                    }
                    result.push_str(" ");
                    result.push_str(&span);
                    has_attributes = true;
                }
                Ok(xmlparser::Token::ElementEnd {
                    end: xmlparser::ElementEnd::Open,
                    ..
                }) => {
                    // ">"
                    if has_attributes {
                        result.push_str("&gt;");
                    } else {
                        result.push_str("</b>&gt;");
                    }
                    indent += 1;
                    has_text = false;
                }
                Ok(xmlparser::Token::ElementEnd {
                    end: xmlparser::ElementEnd::Empty,
                    ..
                }) =>
                // "/>"
                {
                    if has_attributes {
                        result.push_str("/&gt;");
                    } else {
                        result.push_str("</b>/&gt;");
                    }
                }
                Ok(xmlparser::Token::ElementEnd {
                    end: xmlparser::ElementEnd::Close(_, name),
                    ..
                }) => {
                    // </name>
                    indent -= 1;
                    if !has_text {
                        result.push_str("\n");
                        for _ in 0..indent {
                            result.push_str("  ");
                        }
                    }
                    result.push_str("&lt;/<b>");
                    result.push_str(&name);
                    result.push_str("</b>&gt;");
                    has_text = false;
                }
                Ok(xmlparser::Token::Text { text, .. }) => {
                    let txt = text.replace("\n", "").trim().to_string();
                    if !txt.is_empty() {
                        result.push_str(&txt);
                        has_text = true;
                    }
                }
                Ok(xmlparser::Token::Declaration { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::ProcessingInstruction { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::Comment { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::DtdStart { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::EmptyDtd { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::DtdEnd { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::EntityDeclaration { span, .. }) => {
                    result.push_str(&span);
                }
                Ok(xmlparser::Token::Cdata { span, .. }) => {
                    result.push_str(&span);
                }
                Err(_) => return xml.to_string(),
            }
        }
        result
    }

    fn highlight_indent_json(json: &str) -> String {
        if let Ok(val) = serde_json::from_str(json) {
            Self::highlight_indent_json_value(&val, 0)
        } else {
            json.to_string()
        }
    }

    fn highlight_indent_json_value(v: &serde_json::Value, indent_depth: usize) -> String {
        let next_indent = " ".repeat((indent_depth + 1) * 2);
        let cur_indent = &next_indent[0..(next_indent.len() - 2)];
        match v {
            serde_json::Value::Object(fields) => {
                cur_indent.to_string()
                    + "{"
                    + &fields
                        .iter()
                        .map(|(k, v)| {
                            format!(
                                "\n{}\"<b>{}</b>\": {}",
                                next_indent,
                                k,
                                Self::highlight_indent_json_value(v, indent_depth + 1)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                    + "\n"
                    + cur_indent
                    + "}"
            }
            serde_json::Value::Array(entries) => {
                cur_indent.to_string()
                    + "["
                    + &entries
                        .iter()
                        .map(|e| {
                            format!(
                                "\n{}{}",
                                &next_indent,
                                Self::highlight_indent_json_value(e, indent_depth + 1)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                    + "\n"
                    + cur_indent
                    + "]"
            }
            _ => v.to_string(),
        }
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            margin_top: 10,
            margin_bottom: 10,
            margin_start: 10,
            margin_end: 10,
            spacing: 10,
            #[name="comm_info_header"]
            CommInfoHeader(self.model.client_ip.clone(), self.model.stream_id) {
            },
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.request.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing request info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: &self.model.data.request.as_ref().map(|r| r.other_lines.as_str()).unwrap_or(""),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                markup: &Self::highlight_indent(
                    self.model.data.request.as_ref().and_then(|r| r.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                    self.model.data.request.as_ref().and_then(|r| r.content_type.as_deref())),
                xalign: 0.0,
                visible: self.model.data.request.as_ref().and_then(|r| r.body.as_ref()).is_some(),
                selectable: true,
            },
            gtk::Separator {},
            #[style_class="http_first_line"]
            gtk::Label {
                label: &self.model.data.response.as_ref().map(|r| r.first_line.as_str()).unwrap_or("Missing response info"),
                xalign: 0.0,
                selectable: true,
            },
            gtk::Label {
                label: &self.model.data.response.as_ref().map(|r| r.other_lines.as_str()).unwrap_or(""),
                xalign: 0.0,
                selectable: true,
            },
            #[name="contents_stack"]
            gtk::Stack {
                gtk::Label {
                    child: {
                        name: Some(TEXT_CONTENTS_STACK_NAME)
                    },
                    markup: &Self::highlight_indent(
                        self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                        self.model.data.response.as_ref().and_then(|r| r.content_type.as_deref())),
                    xalign: 0.0,
                    visible: self.model.data.response.as_ref().and_then(|r| r.body.as_ref()).is_some(),
                    selectable: true,
                },
                #[name="body_image"]
                gtk::Image {
                    child: {
                        name: Some(IMAGE_CONTENTS_STACK_NAME)
                    },
                }
            }
        }
    }
}

#[test]
fn simple_xml_indent() {
    assert_eq!(
        "<?xml?>\n&lt;<b>body</b>&gt;\n  &lt;<b>tag1</b>/&gt;\n  &lt;<b>tag2</b> attr=\"val\"&gt;contents&lt;/<b>tag2</b>&gt;\n&lt;/<b>body</b>&gt;",
        HttpCommEntry::highlight_indent_xml("<?xml?><body><tag1/><tag2 attr=\"val\">contents</tag2></body>")
    );
}

#[test]
fn simple_xml_indent_already_indented() {
    assert_eq!(
        "<?xml?>\n&lt;<b>body</b>&gt;\n  &lt;<b>tag1</b>/&gt;\n  &lt;<b>tag2</b> attr=\"val\"&gt;contents&lt;/<b>tag2</b>&gt;\n&lt;/<b>body</b>&gt;",
        HttpCommEntry::highlight_indent_xml("<?xml?>\n<body>\n\n\n      <tag1/>\n\n\n<tag2 attr=\"val\">contents</tag2>\n</body>")
    );
}

#[test]
fn xml_highlight_attrs_no_children() {
    assert_eq!(
        "&lt;<b>mytag</b> attr1=\"a\" attr2=\"b\"/&gt;",
        HttpCommEntry::highlight_indent_xml("<mytag attr1=\"a\" attr2=\"b\" />")
    );
}

#[test]
fn xml_indent_long_lines() {
    assert_eq!(
        "&lt;<b>mytag</b> firstattr=\"first value\" secondattr=\"second value\" thirdattr=\"third value\"\n   fourthattr=\"fourth value\" fifthattr=\"fifth value\"/&gt;",
        HttpCommEntry::highlight_indent_xml("<mytag firstattr=\"first value\" secondattr=\"second value\" thirdattr=\"third value\" fourthattr=\"fourth value\" fifthattr=\"fifth value\"/>"))
}

#[test]
fn simple_json_indent() {
    assert_eq!(
        "{\n  \"<b>field1</b>\": 12,\n  \"<b>field2</b>\": [\n    \"hi\"\n,    \"array\",  ]\n}",
        HttpCommEntry::highlight_indent_json(r#"{"field1": 12, "field2": ["hi", "array"]}"#)
    );
}
