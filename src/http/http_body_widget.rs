use super::code_formatting;
use super::http_message_parser;
use super::http_message_parser::HttpRequestResponseData;
use crate::tshark_communication_raw::TSharkCommunicationRaw;
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
    FormatCodeChanged(bool),
    GotImage(Vec<u8>, u32),
    RequestResponseChanged(Option<HttpRequestResponseData>, PathBuf),
}

pub struct Model {
    format_code: bool,
    data: Option<HttpRequestResponseData>,
    bg_sender: mpsc::Sender<BgFunc>,

    _got_image_channel: relm::Channel<(Vec<u8>, u32)>,
    got_image_sender: relm::Sender<(Vec<u8>, u32)>,
}

#[widget]
impl Widget for HttpBodyWidget {
    fn model(relm: &relm::Relm<Self>, bg_sender: mpsc::Sender<BgFunc>) -> Model {
        let (_got_image_channel, got_image_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |d: (Vec<u8>, u32)| stream.emit(Msg::GotImage(d.0, d.1)))
        };
        Model {
            format_code: true,
            bg_sender,
            data: None,
            _got_image_channel,
            got_image_sender,
        }
    }

    fn update(&mut self, event: Msg) {
        dbg!(&event);
        match event {
            Msg::FormatCodeChanged(format_code) => {
                self.model.format_code = format_code;
            }
            Msg::RequestResponseChanged(http_data, file_path) => {
                let content_length = http_data
                    .as_ref()
                    .and_then(|d| {
                        http_message_parser::get_http_header_value(&d.other_lines, "Content-Length")
                    })
                    .and_then(|l| l.parse::<usize>().ok());
                let body_length = http_data
                    .as_ref()
                    .and_then(|d| d.body.as_ref())
                    .map(|b| b.len());
                let heuristic_is_binary_body = matches!((content_length, body_length),
                    (Some(l1), Some(l2)) if l1 != l2);
                println!(
                    "CL H: {:?} A: {:?}, binary: {}",
                    content_length, body_length, heuristic_is_binary_body
                );
                // it's very important to set the model right now,
                // because setting it resets the contents stack
                // => need to reset the stack to display the "text"
                // child after that, if needed.
                self.model.data = http_data.clone();
                match (
                    &http_data.as_ref().and_then(|d| d.content_type.as_deref()),
                    &http_data.as_ref().and_then(|d| d.body.as_ref()),
                ) {
                    (Some(content_type), Some(body)) if content_type.starts_with("image/") => {
                        let seq_no = http_data.as_ref().unwrap().tcp_seq_number;
                        let s = self.model.got_image_sender.clone();
                        self.model
                            .bg_sender
                            .send(BgFunc::new(move || {
                                Self::load_image(&file_path, seq_no, s.clone())
                            }))
                            .unwrap();
                    }
                    _ => {
                        println!("activate text stack");
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(TEXT_CONTENTS_STACK_NAME);
                    }
                }
            }
            Msg::GotImage(bytes, seq_no) => {
                if self.model.data.as_ref().map(|d| d.tcp_seq_number) == Some(seq_no) {
                    let loader = gdk_pixbuf::PixbufLoader::new();
                    loader.write(&bytes).unwrap();
                    loader.close().unwrap();
                    self.widgets
                        .body_image
                        .set_from_pixbuf(loader.get_pixbuf().as_ref());
                    println!("activate image stack");
                    self.widgets
                        .contents_stack
                        .set_visible_child_name(IMAGE_CONTENTS_STACK_NAME);
                }
            }
        }
    }

    fn load_image(file_path: &Path, tcp_seq_number: u32, s: relm::Sender<(Vec<u8>, u32)>) {
        let mut packets = win::invoke_tshark::<TSharkCommunicationRaw>(
            file_path,
            win::TSharkMode::JsonRaw,
            &format!("tcp.seq eq {}", tcp_seq_number),
        )
        .expect("tshark error");
        if packets.len() == 1 {
            let bytes = packets.pop().unwrap().source.layers.http.unwrap().file_data;
            s.send((bytes, tcp_seq_number)).unwrap();
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
               markup: &code_formatting::highlight_indent(
                   self.model.format_code,
                   self.model.data.as_ref().and_then(|d| d.body.as_ref()).map(|b| b.as_str()).unwrap_or(""),
                   self.model.data.as_ref().and_then(|d| d.content_type.as_deref())),
               xalign: 0.0,
               visible: self.model.data.as_ref().and_then(|d|d.body.as_ref()).is_some(),
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
