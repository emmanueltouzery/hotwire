use super::postgres_message_parser::PostgresMessageData;
use crate::message_parser::MessageInfo;
use crate::pgsql::tshark_pgsql::PostgresColType;
use crate::widgets::comm_info_header;
use crate::widgets::comm_info_header::CommInfoHeader;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use chrono::NaiveDateTime;
use gtk::prelude::*;
use itertools::Itertools;
use regex::Regex;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::borrow::Cow;
use std::fs::File;
use std::io::BufWriter;
use std::io::Write;
use std::net::IpAddr;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;

pub struct Model {
    bg_sender: mpsc::Sender<BgFunc>,
    win_msg_sender: relm::StreamHandle<win::Msg>,
    stream_id: u32,
    client_ip: IpAddr,
    data: PostgresMessageData,
    list_store: Option<gtk::ListStore>,
    syntax_highlight: Vec<(Regex, String)>,

    _saved_resultset_channel: relm::Channel<Option<String>>, // None on success, or error message
    saved_resultset_sender: relm::Sender<Option<String>>,
}

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, MessageInfo),
    ExportResultSet,
}

#[widget]
impl Widget for PostgresCommEntry {
    fn init_view(&mut self) {}

    fn model(
        _relm: &relm::Relm<Self>,
        params: (
            u32,
            IpAddr,
            PostgresMessageData,
            relm::StreamHandle<win::Msg>,
            mpsc::Sender<BgFunc>,
        ),
    ) -> Model {
        let (stream_id, client_ip, data, win_msg_sender, bg_sender) = params;
        let (_saved_resultset_channel, saved_resultset_sender) = {
            let win_stream = win_msg_sender.clone();
            relm::Channel::new(move |d: Option<String>| {
                if d.is_some() {
                    // error
                    win_stream.emit(win::Msg::InfoBarShow(
                        d,
                        win::InfobarOptions::ShowCloseButton,
                    ))
                } else {
                    // success
                    win_stream.emit(win::Msg::InfoBarShow(
                        Some("The export has completed successfully".to_string()),
                        win::InfobarOptions::TimeLimitedWithCloseButton,
                    ))
                }
            })
        };
        Model {
            bg_sender,
            win_msg_sender,
            data,
            stream_id,
            client_ip,
            list_store: None,
            syntax_highlight: Self::prepare_syntax_highlight(),

            saved_resultset_sender,
            _saved_resultset_channel,
        }
    }

    fn prepare_syntax_highlight() -> Vec<(Regex, String)> {
        [
            "select",
            "SELECT",
            "update",
            "UPDATE",
            "delete",
            "DELETE",
            "from",
            "FROM",
            "set",
            "SET",
            "join",
            "JOIN",
            "on",
            "ON",
            "where",
            "WHERE",
            "having",
            "HAVING",
            "group by",
            "GROUP BY",
            "using",
            "USING",
            "order by",
            "ORDER BY",
            "desc",
            "DESC",
            "asc",
            "ASC",
            "limit",
            "LIMIT",
            "not",
            "NOT",
            "in",
            "IN",
            "and",
            "AND",
            "or",
            "OR",
            "inner",
            "INNER",
            "left outer",
            "LEFT OUTER",
            "outer",
            "OUTER",
        ]
        .iter()
        .map(|s| {
            (
                Regex::new(&format!(r"\b{}\b", s)).unwrap(),
                format!("<b>{}</b>", s),
            )
        })
        .collect()
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::DisplayDetails(
                _bg_sender,
                MessageInfo {
                    stream_id,
                    client_ip,
                    message_data: MessageData::Postgres(msg),
                },
            ) => {
                self.model.data = msg;
                self.streams
                    .comm_info_header
                    .emit(comm_info_header::Msg::Update(client_ip, stream_id));
                self.model.stream_id = stream_id;
                self.model.client_ip = client_ip;

                self.fill_resultset();
            }
            Msg::DisplayDetails(_, _) => {}
            Msg::ExportResultSet => {
                let dialog = gtk::FileChooserNativeBuilder::new()
                    .action(gtk::FileChooserAction::Save)
                    .title("Export to...")
                    .do_overwrite_confirmation(true)
                    .modal(true)
                    .build();
                dialog.set_current_name("resultset.csv");
                if dialog.run() == gtk::ResponseType::Accept {
                    let target_fname = dialog.get_filename().unwrap(); // ## unwrap
                    self.model.win_msg_sender.emit(win::Msg::InfoBarShow(
                        Some(format!(
                            "Saving to file {}",
                            &target_fname.to_string_lossy()
                        )),
                        win::InfobarOptions::ShowSpinner,
                    ));
                    {
                        let s = self.model.saved_resultset_sender.clone();
                        let pg_data = self.model.data.clone();
                        self.model
                            .bg_sender
                            .send(BgFunc::new(move || {
                                s.send(
                                    Self::save_resultset(&target_fname, &pg_data)
                                        .map_err(|e| e.to_string())
                                        .err(),
                                )
                                .unwrap()
                            }))
                            .unwrap();
                    }
                }
            }
        }
    }

    fn fill_resultset(&mut self) {
        let field_descs: Vec<_> = self
            .model
            .data
            .resultset_col_types
            .iter()
            .map(|t| match t {
                // I'd love to "optimize" the liststore by storing ints as ints and not
                // as strings. Sadly... https://gtk-rs.org/docs/glib/value/struct.Value.html
                // "Some types (e.g. String and objects) support None values while others (e.g. numeric types) don't."
                //
                // And obviously I want to support 'null'. Therefore write all the columns as strings in the liststore.

                // PostgresColType::Bool => bool::static_type(),
                // PostgresColType::Int2 | PostgresColType::Int4 => i32::static_type(),
                // // PostgresColType::Int8 | PostgresColType::Timestamp => i64::static_type(),
                // PostgresColType::Int8 => i64::static_type(),
                _ => String::static_type(),
            })
            .collect();
        let descs = if field_descs.is_empty() {
            // gtk really doesn't like if there are no columns (crashes or something like that)
            vec![String::static_type()]
        } else {
            field_descs
        };

        let list_store = gtk::ListStore::new(&descs);
        for col in &self.widgets.resultset.get_columns() {
            self.widgets.resultset.remove_column(col);
        }

        for (idx, col_name) in self.model.data.resultset_col_names.iter().enumerate() {
            let col1 = gtk::TreeViewColumnBuilder::new().title(&col_name).build();
            let cell_r_txt = gtk::CellRendererText::new();
            col1.pack_start(&cell_r_txt, true);
            col1.add_attribute(&cell_r_txt, "text", idx as i32);
            self.widgets.resultset.append_column(&col1);
        }

        for row_idx in 0..self.model.data.resultset_row_count {
            let iter = list_store.append();
            Self::visit_resultset_row(
                &self.model.data,
                row_idx,
                &mut (),
                |col_idx, bool_val, _| {
                    list_store.set_value(
                        &iter,
                        col_idx as u32,
                        &bool_val
                            .map(|v| Cow::Owned(v.to_string()))
                            .unwrap_or(Cow::Borrowed("null"))
                            .to_value(),
                    );
                },
                |col_idx, int_val, _| {
                    list_store.set_value(
                        &iter,
                        col_idx as u32,
                        &int_val
                            .map(|v| Cow::Owned(v.to_string()))
                            .unwrap_or(Cow::Borrowed("null"))
                            .to_value(),
                    );
                },
                |col_idx, str_val, _| {
                    list_store.set_value(
                        &iter,
                        col_idx as u32,
                        &str_val.as_deref().unwrap_or("null").to_value(),
                    );
                },
                |col_idx, datetime_val, _| {
                    list_store.set_value(
                        &iter,
                        col_idx as u32,
                        &datetime_val
                            .map(|d| d.to_string())
                            .as_deref()
                            .unwrap_or("null")
                            .to_value(),
                    );
                },
            );
        }
        self.widgets.resultset.set_model(Some(&list_store));
        self.model.list_store = Some(list_store);
    }

    fn visit_resultset_row<D>(
        pg_message: &PostgresMessageData,
        row_idx: usize,
        data: &mut D,
        bool_value_cb: impl Fn(usize, Option<bool>, &mut D),
        int_value_cb: impl Fn(usize, Option<i64>, &mut D),
        string_value_cb: impl Fn(usize, Option<&str>, &mut D),
        datetime_value_cb: impl Fn(usize, Option<NaiveDateTime>, &mut D),
    ) {
        let mut bool_idx = 0;
        let mut int_idx = 0;
        let mut datetime_idx = 0;
        let mut bigint_idx = 0;
        let mut str_idx = 0;
        for (col_idx, col_type) in pg_message.resultset_col_types.iter().enumerate() {
            match col_type {
                PostgresColType::Bool => {
                    bool_value_cb(
                        col_idx,
                        pg_message.resultset_bool_cols[bool_idx][row_idx],
                        data,
                    );
                    bool_idx += 1;
                }
                PostgresColType::Int2 | PostgresColType::Int4 => {
                    int_value_cb(
                        col_idx,
                        pg_message.resultset_int_cols[int_idx][row_idx].map(|i| i as i64),
                        data,
                    );
                    int_idx += 1;
                }
                PostgresColType::Timestamp => {
                    datetime_value_cb(
                        col_idx,
                        pg_message.resultset_datetime_cols[datetime_idx][row_idx],
                        data,
                    );
                    datetime_idx += 1;
                }
                PostgresColType::Int8 => {
                    int_value_cb(
                        col_idx,
                        pg_message.resultset_bigint_cols[bigint_idx][row_idx],
                        data,
                    );
                    bigint_idx += 1;
                }
                _ => {
                    string_value_cb(
                        col_idx,
                        pg_message.resultset_string_cols[str_idx][row_idx].as_deref(),
                        data,
                    );
                    str_idx += 1;
                }
            }
        }
    }

    fn save_resultset(
        target_fname: &Path,
        pg_message: &PostgresMessageData,
    ) -> std::io::Result<()> {
        let mut file = BufWriter::new(File::create(target_fname)?);
        file.write_all(pg_message.resultset_col_names.join("\t").as_bytes())?;
        let write_separator = |s: &mut String, col_idx| {
            if col_idx > 0 {
                s.push('\t');
            } else {
                s.push('\n');
            }
        };
        let mut output = String::new();
        for row_idx in 0..pg_message.resultset_row_count {
            Self::visit_resultset_row(
                pg_message,
                row_idx,
                &mut output,
                |col_idx, bool_val, output| {
                    write_separator(output, col_idx);
                    output.push_str(match bool_val {
                        Some(true) => "true",
                        Some(false) => "false",
                        None => "null",
                    });
                },
                |col_idx, int_val, output| {
                    write_separator(output, col_idx);
                    if let Some(i) = int_val {
                        output.push_str(&i.to_string());
                    } else {
                        output.push_str("null");
                    }
                },
                |col_idx, str_val, output| {
                    write_separator(output, col_idx);
                    output.push_str(str_val.unwrap_or("null"));
                },
                |col_idx, datetime_val, output| {
                    write_separator(output, col_idx);
                    output.push_str(
                        datetime_val
                            .map(|d| d.to_string())
                            .as_deref()
                            .unwrap_or("null"),
                    );
                },
            );
        }
        file.write_all(output.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    fn highlight_sql(highlight: &[(Regex, String)], query: &str) -> String {
        let result = glib::markup_escape_text(query).to_string();
        highlight.iter().fold(result, |sofar, (regex, repl)| {
            regex.replace_all(&sofar, repl).to_string()
        })
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
            gtk::Paned {
                orientation: gtk::Orientation::Vertical,
                gtk::ScrolledWindow {
                    gtk::Box {
                        orientation: gtk::Orientation::Vertical,
                        gtk::Label {
                            markup: &Self::highlight_sql(
                                &self.model.syntax_highlight,
                                self.model.data.query.as_deref().unwrap_or("Failed retrieving the query string")),
                            line_wrap: true,
                            xalign: 0.0,
                            selectable: true,
                        },
                        gtk::Label {
                            markup: &self.model.data.parameter_values
                                                    .iter()
                                                    .cloned()
                                                    .enumerate()
                                                    .map(|(i, p)| format!("<b>${}</b>: {}", i+1, p))
                                                    .intersperse("\n".to_string()).collect::<String>(),
                            visible: !self.model.data.parameter_values.is_empty(),
                            xalign: 0.0,
                        },
                    }
                },
                gtk::Box {
                    orientation: gtk::Orientation::Vertical,
                    gtk::Box {
                        orientation: gtk::Orientation::Horizontal,
                        visible: self.model.data.resultset_row_count > 0,
                        gtk::Label {
                            label: &self.model.data.resultset_row_count.to_string(),
                            xalign: 0.0,
                        },
                        gtk::Label {
                            label: " row(s)",
                            xalign: 0.0,
                        },
                        gtk::Button {
                            child: {
                                pack_type: gtk::PackType::End,
                            },
                            always_show_image: true,
                            image: Some(&gtk::Image::from_icon_name(
                                Some("document-save-symbolic"), gtk::IconSize::Menu)),
                            label: "Export resultset...",
                            button_press_event(_, _) => (Msg::ExportResultSet, Inhibit(false)),
                        }
                    },
                    gtk::ScrolledWindow {
                        #[name="resultset"]
                        gtk::TreeView {
                            hexpand: true,
                            vexpand: true,
                            visible: !self.model.data.resultset_row_count > 0
                        },
                    }
                }
            }
        }
    }
}
