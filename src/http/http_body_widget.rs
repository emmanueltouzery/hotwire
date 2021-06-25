use super::code_formatting;
use super::http_message_parser;
use super::http_message_parser::{HttpBody, HttpRequestResponseData};
use crate::widgets::win;
use crate::BgFunc;
use gdk_pixbuf::prelude::*;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::borrow::Cow;
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
    RequestResponseChanged {
        http_data: Option<HttpRequestResponseData>,
        request_first_line_if_response: Option<String>,
    },
    SaveContents,
}

pub struct Model {
    win_msg_sender: relm::StreamHandle<win::Msg>,

    format_code: bool,
    data: Option<HttpRequestResponseData>,

    request_first_line_if_response: Option<String>,

    _saved_body_channel: relm::Channel<SavedBodyData>,
    saved_body_sender: relm::Sender<SavedBodyData>,
}

#[widget]
impl Widget for HttpBodyWidget {
    fn init_view(&mut self) {
        self.widgets.too_long_infobar.get_content_area().add(
            &gtk::LabelBuilder::new()
                .label("The message body is too large, has been truncated for display")
                .build(),
        );
    }

    fn model(
        _relm: &relm::Relm<Self>,
        params: (relm::StreamHandle<win::Msg>, mpsc::Sender<BgFunc>),
    ) -> Model {
        let (win_msg_sender, _bg_sender) = params;
        let (_saved_body_channel, saved_body_sender) = {
            let win_stream = win_msg_sender.clone();
            relm::Channel::new(move |d: SavedBodyData| {
                win_stream.emit(win::Msg::InfoBarShow(
                    d.error_msg,
                    win::InfobarOptions::ShowCloseButton,
                ))
            })
        };
        Model {
            win_msg_sender,
            format_code: true,
            data: None,
            request_first_line_if_response: None,
            _saved_body_channel,
            saved_body_sender,
        }
    }

    fn update(&mut self, event: Msg) {
        // dbg!(&event);
        match event {
            Msg::FormatCodeChanged(format_code) => {
                self.model.format_code = format_code;
            }
            Msg::RequestResponseChanged {
                http_data,
                request_first_line_if_response,
            } => {
                // it's very important to set the model right now,
                // because setting it resets the contents stack
                // => need to reset the stack to display the "text"
                // child after that, if needed.
                self.model.data = http_data.clone();
                self.model.request_first_line_if_response = request_first_line_if_response;

                // need to try to decode as string.. the content-type may not be
                // populated or be too exotic, and binary contents don't mean much
                // if the data is encoded as brotli, gzip and so on
                let is_data_str = http_data.as_ref().and_then(|d| d.body_as_str()).is_some();

                match (
                    &http_data.as_ref().and_then(|d| d.content_type.as_deref()),
                    &http_data.as_ref().map(|d| &d.body),
                    is_data_str,
                ) {
                    (Some(content_type), Some(HttpBody::Binary(bytes)), false)
                        if content_type.starts_with("image/") =>
                    {
                        self.display_image(bytes);
                    }
                    (_, _, false) => {
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(BINARY_CONTENTS_STACK_NAME);
                    }
                    _ => {
                        self.widgets.too_long_header.set_visible(
                            self.model
                                .data
                                .as_ref()
                                .and_then(|d| d.body_as_str())
                                .unwrap_or(Cow::Borrowed(""))
                                .len()
                                > code_formatting::BODY_TRUNCATE_LIMIT_BYTES,
                        );
                        self.widgets
                            .contents_stack
                            .set_visible_child_name(TEXT_CONTENTS_STACK_NAME);
                    }
                }
            }
            Msg::SaveContents => {
                let dialog = gtk::FileChooserNativeBuilder::new()
                    .action(gtk::FileChooserAction::Save)
                    .title("Export to...")
                    .do_overwrite_confirmation(true)
                    .modal(true)
                    .build();
                dialog.set_current_name(&self.body_save_filename());
                if dialog.run() == gtk::ResponseType::Accept {
                    let target_fname = dialog.get_filename().unwrap(); // ## unwrap
                    self.model.win_msg_sender.emit(win::Msg::InfoBarShow(
                        Some(format!(
                            "Saving to file {}",
                            &target_fname.to_string_lossy()
                        )),
                        win::InfobarOptions::ShowSpinner,
                    ));
                    self.model
                        .saved_body_sender
                        .send(SavedBodyData {
                            error_msg: std::fs::write(
                                target_fname,
                                match self.model.data.as_ref().map(|d| &d.body) {
                                    Some(HttpBody::Binary(ref bytes)) => bytes,
                                    Some(HttpBody::Text(ref txt)) => txt.as_bytes(),
                                    _ => &[],
                                },
                            )
                            .err()
                            .map(|e| e.to_string()),
                        })
                        .unwrap()
                }
            }
        }
    }

    fn display_image(&self, bytes: &[u8]) {
        let loader = gdk_pixbuf::PixbufLoader::new();
        let r = loader.write(bytes);
        loader.close().unwrap();
        if r.is_ok() {
            self.widgets
                .body_image
                .set_from_pixbuf(loader.get_pixbuf().as_ref());
            self.widgets
                .contents_stack
                .set_visible_child_name(IMAGE_CONTENTS_STACK_NAME);
        }
    }

    fn body_save_filename(&self) -> String {
        let attachment_name = self
            .model
            .data
            .as_ref()
            .and_then(|d| {
                http_message_parser::get_http_header_value(&d.headers, "Content-Disposition")
            })
            .and_then(|d| {
                d.strip_prefix("attachment: filename=\"")
                    .and_then(|f| f.strip_suffix("\""))
                    .map(|f| f.to_string())
            });
        attachment_name
            .or_else(|| {
                self.model
                    .request_first_line_if_response
                    .as_deref()
                    .and_then(Self::extract_fname_from_get_request)
            })
            .or_else(|| {
                self.model
                    .data
                    .as_ref()
                    .and_then(|d| d.content_type.as_ref())
                    .and_then(|ct| Self::filename_from_binary_content_type(ct))
            })
            .unwrap_or_else(|| "data.bin".to_string())
    }

    fn extract_fname_from_get_request(line: &str) -> Option<String> {
        let url = match line.split(' ').collect::<Vec<_>>()[..] {
            ["GET", url, "HTTP/1.1"] => Some(url),
            ["GET", url] => Some(url),
            _ => None,
        };
        url.and_then(|u| u.rsplit_once('/'))
            .map(|(_s, fname)| fname.to_string())
            .map(|fname| {
                if let Some((f, _params)) = fname.split_once('?') {
                    f.to_string()
                } else {
                    fname
                }
            })
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

    view! {
       #[name="contents_stack"]
       gtk::Stack {
           visible: self.model.data.as_ref().filter(|d| !matches!(d.body, HttpBody::Missing)).is_some(),
           gtk::Box {
               child: {
                   name: Some(TEXT_CONTENTS_STACK_NAME)
               },
               orientation: gtk::Orientation::Vertical,
               #[name="too_long_header"]
               gtk::Box {
                   #[name="too_long_infobar"]
                   gtk::InfoBar {
                   },
                   gtk::Button {
                       always_show_image: true,
                       image: Some(&gtk::Image::from_icon_name(
                           Some("document-save-symbolic"), gtk::IconSize::Menu)),
                       button_press_event(_, _) => (Msg::SaveContents, Inhibit(false)),
                       label: "Save contents"
                   }
               },
               gtk::ScrolledWindow {
                   property_vscrollbar_policy: gtk::PolicyType::Never,
                   gtk::Label {
                       markup: &code_formatting::highlight_indent_truncate(
                           self.model.format_code,
                           &self.model.data.as_ref().and_then(|d| d.body_as_str()).unwrap_or(Cow::Borrowed("")),
                           self.model.data.as_ref().and_then(|d| d.content_type.as_deref())),
                       xalign: 0.0,
                       selectable: true,
                   },
               }
           },
           gtk::Box {
               child: {
                   name: Some(IMAGE_CONTENTS_STACK_NAME)
               },
               orientation: gtk::Orientation::Vertical,
               #[name="body_image"]
               gtk::Image {
                   halign: gtk::Align::Start,
               },
               gtk::Button {
                   halign: gtk::Align::Start,
                   always_show_image: true,
                   image: Some(&gtk::Image::from_icon_name(
                        Some("document-save-symbolic"), gtk::IconSize::Menu)),
                   button_press_event(_, _) => (Msg::SaveContents, Inhibit(false)),
                   label: "Save image"
               }
           },
           gtk::Box {
               child: {
                   name: Some(BINARY_CONTENTS_STACK_NAME)
               },
               orientation: gtk::Orientation::Vertical,
               gtk::Label {
                   text: "Body contents are binary data",
                   halign: gtk::Align::Start,
               },
               gtk::Button {
                   always_show_image: true,
                   image: Some(&gtk::Image::from_icon_name(
                        Some("document-save-symbolic"), gtk::IconSize::Menu)),
                    button_press_event(_, _) => (Msg::SaveContents, Inhibit(false)),
                   label: "Save body contents",
                   halign: gtk::Align::Start,
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

#[test]
pub fn get_request_to_download_fname_http1() {
    assert_eq!(
        "65879907_fp-us.jpg",
        HttpBodyWidget::extract_fname_from_get_request(
            "GET /_up/upload/2021/04/14/65879907_fp-us.jpg HTTP/1.1"
        )
        .unwrap()
    );
}

#[test]
pub fn get_request_to_download_fname_http2_question_mark() {
    assert_eq!("redot.gif",
HttpBodyWidget::extract_fname_from_get_request(
    "GET /_16189379808800/redot.gif?l=1&id=cthA3c_qM8KyoQ2BLdAWjqQPLU7G3Jss8tN5ZbOjVHf.J7&arg=0&sarg=OZS%3A%").unwrap());
}
