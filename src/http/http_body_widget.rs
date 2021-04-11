use super::http_message_parser;
use super::http_message_parser::HttpRequestResponseData;
use crate::tshark_communication_raw::TSharkCommunicationRaw;
use crate::widgets::win;
use gdk_pixbuf::prelude::*;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::path::Path;

const TEXT_CONTENTS_STACK_NAME: &str = "text";
const IMAGE_CONTENTS_STACK_NAME: &str = "image";

#[derive(Msg, Debug)]
pub enum Msg {
    FormatCodeChanged(bool),
    GotImage(Vec<u8>),
    RequestResponseChanged(HttpRequestResponseData),
}

pub struct Model {
    format_code: bool,
    data: HttpRequestResponseData,

    _got_image_channel: relm::Channel<Vec<u8>>,
    got_image_sender: relm::Sender<Vec<u8>>,
}

#[widget]
impl Widget for HttpBodyWidget {
    fn model(relm: &relm::Relm<Self>, _: ()) -> Model {
        let (format_request_response, http_data) = params;
        let (_got_image_channel, got_image_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |r: Vec<u8>| stream.emit(Msg::GotImage(r)))
        };
        Model {
            format_code: true,
            data: Default::default(),
            _got_image_channel,
            got_image_sender,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::FormatCodeChanged(format_code) => {
                self.model.format_code = format_code;
            }
            Msg::RequestResponseChanged(http_data) => {
                let content_length = self
                    .model
                    .data
                    .request
                    .as_ref()
                    .and_then(|r| {
                        http_message_parser::get_http_header_value(&r.other_lines, "Content-Length")
                    })
                    .and_then(|l| l.parse::<usize>().ok());
                let body_length = self
                    .model
                    .data
                    .request
                    .as_ref()
                    .and_then(|r| r.body.as_ref())
                    .map(|b| b.len());
                let heuristic_is_binary_body = matches!((content_length, body_length),
                    (Some(l1), Some(l2)) if l1 != l2);
                println!(
                    "CL H: {:?} A: {:?}, binary: {}",
                    content_length, body_length, heuristic_is_binary_body
                );
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
                self.model.http_data = http_data;
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

    view! {
       #[name="contents_stack"]
       gtk::Stack {
           gtk::Label {
               child: {
                   name: Some(TEXT_CONTENTS_STACK_NAME)
               },
               markup: &Self::highlight_indent(
               self.model.format_request_response,
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
