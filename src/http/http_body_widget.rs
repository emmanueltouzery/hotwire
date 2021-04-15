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
const BINARY_CONTENTS_STACK_NAME: &str = "binary";

const KNOWN_CONTENT_TYPE_PREFIXES: &[(&str, &str)] = &[
    ("image/", "image"),
    ("audio/", "audio"),
    ("video/", "video"),
    ("application/", "contents"),
];

#[derive(Debug)]
pub struct SavedBodyData {
    error_msg: Option<String>,
}

#[derive(Msg, Debug)]
pub enum Msg {
    FormatCodeChanged(bool),
    GotImage(Vec<u8>, u32),
    SavedBody(SavedBodyData),
    RequestResponseChanged(Option<HttpRequestResponseData>, PathBuf),
    SaveBinaryContents,
}

pub struct Model {
    format_code: bool,
    data: Option<HttpRequestResponseData>,
    file_path: Option<PathBuf>,
    bg_sender: mpsc::Sender<BgFunc>,

    _got_image_channel: relm::Channel<(Vec<u8>, u32)>,
    got_image_sender: relm::Sender<(Vec<u8>, u32)>,

    _saved_body_channel: relm::Channel<SavedBodyData>,
    saved_body_sender: relm::Sender<SavedBodyData>,
}

#[widget]
impl Widget for HttpBodyWidget {
    fn model(relm: &relm::Relm<Self>, bg_sender: mpsc::Sender<BgFunc>) -> Model {
        let (_got_image_channel, got_image_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |d: (Vec<u8>, u32)| stream.emit(Msg::GotImage(d.0, d.1)))
        };
        let (_saved_body_channel, saved_body_sender) = {
            let stream = relm.stream().clone();
            relm::Channel::new(move |d: SavedBodyData| stream.emit(Msg::SavedBody(d)))
        };
        Model {
            format_code: true,
            bg_sender,
            data: None,
            file_path: None,
            _got_image_channel,
            got_image_sender,
            _saved_body_channel,
            saved_body_sender,
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
                    (Some(l1), Some(l2)) if l1 != l2 && l1 != l2+1); // I've seen content-length==body-length+1 for non-binary
                println!(
                    "CL H: {:?} A: {:?}, binary: {}",
                    content_length, body_length, heuristic_is_binary_body
                );
                // it's very important to set the model right now,
                // because setting it resets the contents stack
                // => need to reset the stack to display the "text"
                // child after that, if needed.
                self.model.data = http_data.clone();
                self.model.file_path = Some(file_path.clone());
                match (
                    &http_data.as_ref().and_then(|d| d.content_type.as_deref()),
                    &http_data.as_ref().and_then(|d| d.body.as_ref()),
                ) {
                    (Some(content_type), Some(body)) if content_type.starts_with("image/") => {
                        let stream_no = http_data.as_ref().unwrap().tcp_stream_no;
                        let seq_no = http_data.as_ref().unwrap().tcp_seq_number;
                        let s = self.model.got_image_sender.clone();
                        self.model
                            .bg_sender
                            .send(BgFunc::new(move || {
                                Self::load_body_bytes(&file_path, stream_no, seq_no, s.clone())
                            }))
                            .unwrap();
                    }
                    _ if heuristic_is_binary_body => {
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(BINARY_CONTENTS_STACK_NAME);
                    }
                    _ => {
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
                    self.widgets
                        .contents_stack
                        .set_visible_child_name(IMAGE_CONTENTS_STACK_NAME);
                }
            }
            Msg::SaveBinaryContents => {
                let dialog = gtk::FileChooserNativeBuilder::new()
                    .action(gtk::FileChooserAction::Save)
                    .title("Export to...")
                    .modal(true)
                    .build();
                dialog.set_current_name(&self.body_save_filename());
                if dialog.run() == gtk::ResponseType::Accept {
                    println!("save {:?}", dialog.get_filename());
                    let stream_no = self.model.data.as_ref().unwrap().tcp_stream_no;
                    let seq_no = self.model.data.as_ref().unwrap().tcp_seq_number;
                    let s = self.model.saved_body_sender.clone();
                    let file_path = self.model.file_path.clone().unwrap();
                    let target_fname = dialog.get_filename().unwrap(); // ## unwrap
                    self.model
                        .bg_sender
                        .send(BgFunc::new(move || {
                            s.send(SavedBodyData {
                                error_msg: Self::save_body_bytes(
                                    &file_path,
                                    stream_no,
                                    seq_no,
                                    &target_fname,
                                ),
                            })
                            .unwrap()
                        }))
                        .unwrap();
                }
            }
            Msg::SavedBody(SavedBodyData { error_msg: None }) => {}
            Msg::SavedBody(SavedBodyData {
                error_msg: Some(err),
            }) => {}
        }
    }

    fn body_save_filename(&self) -> String {
        let attachment_name = self
            .model
            .data
            .as_ref()
            .and_then(|d| {
                http_message_parser::get_http_header_value(&d.other_lines, "Content-Disposition")
            })
            .and_then(|d| {
                d.strip_prefix("attachment: filename=\"")
                    .and_then(|f| f.strip_suffix("\""))
                    .map(|f| f.to_string())
            });
        attachment_name
            .or_else(|| {
                self.model
                    .data
                    .as_ref()
                    .and_then(|d| d.content_type.as_ref())
                    .and_then(|ct| Self::filename_from_binary_content_type(ct))
            })
            .unwrap_or_else(|| "data.bin".to_string())
    }

    fn filename_from_binary_content_type(content_type: &str) -> Option<String> {
        match content_type {
            "image/jpeg" => {
                return Some("image.jpg".to_string());
            }
            "application/msword" => {
                return Some("document.docx".to_string());
            }
            _ => {}
        }
        for (prefix, fname) in KNOWN_CONTENT_TYPE_PREFIXES {
            if let Some(ext) = content_type.strip_prefix(prefix) {
                return Some(format!(
                    "{}.{}",
                    fname,
                    Self::content_type_extension_cleanup(ext)
                ));
            }
        }
        None
    }

    fn content_type_extension_cleanup(extension: &str) -> &str {
        extension
            .trim_start_matches("x-")
            .splitn(2, '+')
            .last()
            .unwrap()
    }

    fn read_body_bytes(file_path: &Path, tcp_stream_id: u32, tcp_seq_number: u32) -> Vec<u8> {
        let mut packets = win::invoke_tshark::<TSharkCommunicationRaw>(
            file_path,
            win::TSharkMode::JsonRaw,
            &format!("tcp.seq_raw eq {} && http", tcp_seq_number),
        )
        .expect("tshark error");
        if packets.len() == 1 {
            packets.pop().unwrap().source.layers.http.unwrap().file_data
        } else {
            panic!(format!(
                "unexpected json from tshark, tcp stream {}, seq number {}",
                tcp_stream_id, tcp_seq_number
            ));
        }
    }

    fn save_body_bytes(
        pcap_file_path: &Path,
        tcp_stream_id: u32,
        tcp_seq_number: u32,
        target_path: &Path,
    ) -> Option<String> {
        let bytes = Self::read_body_bytes(pcap_file_path, tcp_stream_id, tcp_seq_number);
        let r = std::fs::write(target_path, bytes);
        r.err().map(|e| e.to_string())
    }

    fn load_body_bytes(
        file_path: &Path,
        tcp_stream_id: u32,
        tcp_seq_number: u32,
        s: relm::Sender<(Vec<u8>, u32)>,
    ) {
        let bytes = Self::read_body_bytes(file_path, tcp_stream_id, tcp_seq_number);
        s.send((bytes, tcp_seq_number)).unwrap();
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
           },
           gtk::Box {
               child: {
                   name: Some(BINARY_CONTENTS_STACK_NAME)
               },
               gtk::Label {
                   text: "Body contents are binary data"
               },
               gtk::Button {
                   always_show_image: true,
                   image: Some(&gtk::Image::from_icon_name(
                        Some("document-save-symbolic"), gtk::IconSize::Menu)),
                    button_press_event(_, _) => (Msg::SaveBinaryContents, Inhibit(false)),
                   label: "Save body contents"
               }
           }
       }
    }
}

#[test]
pub fn content_type_to_filename_tests() {
    for (ct, fname) in &[
        ("application/zip", "contents.zip"),
        ("application/x-bzip", "contents.bzip"),
        ("audio/aac", "audio.aac"),
        ("image/gif", "image.gif"),
        ("audio/x-midi", "audio.midi"),
        ("application/epub+zip", "contents.zip"),
        ("application/msword", "document.docx"),
    ] {
        assert_eq!(
            Some(*fname),
            HttpBodyWidget::filename_from_binary_content_type(ct).as_deref()
        );
    }
}
