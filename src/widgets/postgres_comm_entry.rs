use super::postgres_parsing;
use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::BgFunc;
use crate::TSharkCommunication;
use chrono::{NaiveDateTime, Utc};
use gtk::prelude::*;
use itertools::Itertools;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::sync::mpsc;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.pgsql.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::DATABASE
    }

    fn parse_stream(&self, stream: &[TSharkCommunication]) -> Vec<MessageData> {
        let mut all_vals = vec![];
        for msg in stream {
            let root = msg.source.layers.pgsql.as_ref();
            match root {
                Some(serde_json::Value::Object(_)) => all_vals.push((msg, root.unwrap())),
                Some(serde_json::Value::Array(vals)) => {
                    all_vals.extend(vals.iter().map(|v| (msg, v)))
                }
                _ => {}
            }
        }
        postgres_parsing::parse_pg_stream(all_vals)
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            String::static_type(), // query first line
            String::static_type(), // response info (number of rows..)
            u32::static_type(),    // stream_id
            u32::static_type(),    // index of the comm in the model vector
            String::static_type(), // query start timestamp (string)
            i64::static_type(),    // query start timestamp (integer, for sorting)
            i32::static_type(),    // query duration (nanos, for sorting)
            String::static_type(), // query duration display
        ]);

        let timestamp_col = gtk::TreeViewColumnBuilder::new()
            .title("Timestamp")
            .resizable(true)
            .sort_column_id(5)
            .build();
        let cell_t_txt = gtk::CellRendererTextBuilder::new().build();
        timestamp_col.pack_start(&cell_t_txt, true);
        timestamp_col.add_attribute(&cell_t_txt, "text", 4);
        tv.append_column(&timestamp_col);

        let query_col = gtk::TreeViewColumnBuilder::new()
            .title("Query")
            .expand(true)
            .resizable(true)
            .build();
        let cell_q_txt = gtk::CellRendererTextBuilder::new()
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        query_col.pack_start(&cell_q_txt, true);
        query_col.add_attribute(&cell_q_txt, "text", 0);
        tv.append_column(&query_col);

        let result_col = gtk::TreeViewColumnBuilder::new()
            .title("Result")
            .resizable(true)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new().build();
        result_col.pack_start(&cell_r_txt, true);
        result_col.add_attribute(&cell_r_txt, "text", 1);
        tv.append_column(&result_col);

        let duration_col = gtk::TreeViewColumnBuilder::new()
            .title("Duration")
            .resizable(true)
            .sort_column_id(6)
            .build();
        let cell_d_txt = gtk::CellRendererTextBuilder::new().build();
        duration_col.pack_start(&cell_d_txt, true);
        duration_col.add_attribute(&cell_d_txt, "text", 7);
        tv.append_column(&duration_col);

        let model_sort = gtk::TreeModelSort::new(&liststore);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));

        liststore
    }

    fn populate_treeview(&self, ls: &gtk::ListStore, session_id: u32, messages: &Vec<MessageData>) {
        for (idx, message) in messages.iter().enumerate() {
            let iter = ls.append();
            let postgres = message.as_postgres().unwrap();
            ls.set_value(
                &iter,
                0,
                &postgres
                    .query
                    .as_deref()
                    .map(|q| if q.len() > 250 { &q[..250] } else { q })
                    .unwrap_or("couldn't get query")
                    .replace("\n", "")
                    .to_value(),
            );
            ls.set_value(
                &iter,
                1,
                &format!("{} rows", postgres.resultset_row_count).to_value(),
            );
            ls.set_value(&iter, 2, &session_id.to_value());
            ls.set_value(&iter, 3, &(idx as i32).to_value());
            ls.set_value(&iter, 4, &postgres.query_timestamp.to_string().to_value());
            ls.set_value(
                &iter,
                5,
                &postgres.query_timestamp.timestamp_nanos().to_value(),
            );
            ls.set_value(
                &iter,
                6,
                &(postgres.result_timestamp - postgres.query_timestamp)
                    .num_milliseconds()
                    .to_value(),
            );
            ls.set_value(
                &iter,
                7,
                &format!(
                    "{} ms",
                    (postgres.result_timestamp - postgres.query_timestamp).num_milliseconds()
                )
                .to_value(),
            );
        }
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        _bg_sender: mpsc::Sender<BgFunc>,
    ) -> relm::StreamHandle<MessageParserDetailsMsg> {
        let component = Box::leak(Box::new(parent.add_widget::<PostgresCommEntry>(
            PostgresMessageData {
                query: None,
                query_timestamp: Utc::now().naive_local(),
                result_timestamp: Utc::now().naive_local(),
                parameter_values: vec![],
                resultset_col_names: vec![],
                resultset_row_count: 0,
                resultset_first_rows: vec![],
            },
        )));
        component.stream()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostgresMessageData {
    // for prepared queries, it's possible the declaration
    // occured before we started recording the stream.
    // in that case we won't be able to recover the query string.
    pub query_timestamp: NaiveDateTime,
    pub result_timestamp: NaiveDateTime,
    pub query: Option<String>,
    pub parameter_values: Vec<String>,
    pub resultset_col_names: Vec<String>,
    pub resultset_row_count: usize,
    pub resultset_first_rows: Vec<Vec<String>>,
}

pub struct Model {
    data: PostgresMessageData,
    list_store: Option<gtk::ListStore>,
}

#[widget]
impl Widget for PostgresCommEntry {
    fn init_view(&mut self) {}

    fn model(relm: &relm::Relm<Self>, data: PostgresMessageData) -> Model {
        let field_descs: Vec<_> = data
            .resultset_first_rows
            .first()
            .filter(|r| !r.is_empty()) // list store can't have 0 columns
            .map(|r| vec![String::static_type(); r.len()])
            // the list store can't have 0 columns, put one String by default
            .unwrap_or_else(|| vec![String::static_type()]);

        Model {
            data,
            list_store: None,
        }
    }

    fn update(&mut self, event: MessageParserDetailsMsg) {
        match event {
            MessageParserDetailsMsg::DisplayDetails(
                _bg_sender,
                _path,
                MessageData::Postgres(msg),
            ) => {
                self.model.data = msg;

                let field_descs: Vec<_> = self
                    .model
                    .data
                    .resultset_first_rows
                    .first()
                    .filter(|r| !r.is_empty()) // list store can't have 0 columns
                    .map(|r| vec![String::static_type(); r.len()])
                    // the list store can't have 0 columns, put one String by default
                    .unwrap_or_else(|| vec![String::static_type()]);

                let list_store = gtk::ListStore::new(&field_descs);
                // println!("{:?}", self.model.data.query);
                // println!("{:?}", self.model.data.resultset_first_rows);
                for col in &self.widgets.resultset.get_columns() {
                    self.widgets.resultset.remove_column(col);
                }
                if let Some(first) = self.model.data.resultset_first_rows.first() {
                    // println!("first len {}", first.len());
                    for i in 0..first.len() {
                        let col1 = gtk::TreeViewColumnBuilder::new()
                            .title(
                                self.model
                                    .data
                                    .resultset_col_names
                                    .get(i)
                                    .map(|s| s.as_str())
                                    .unwrap_or("Col"),
                            )
                            .build();
                        let cell_r_txt = gtk::CellRendererText::new();
                        col1.pack_start(&cell_r_txt, true);
                        col1.add_attribute(&cell_r_txt, "text", i as i32);
                        self.widgets.resultset.append_column(&col1);
                    }
                }
                for row in &self.model.data.resultset_first_rows {
                    let iter = list_store.append();
                    for (col_idx, col) in row.iter().enumerate() {
                        list_store.set_value(&iter, col_idx as u32, &col.to_value());
                    }
                }
                self.widgets.resultset.set_model(Some(&list_store));
                self.model.list_store = Some(list_store);
            }
            _ => {}
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
            #[style_class="http_first_line"]
            gtk::Label {
                label: self.model.data.query.as_deref().unwrap_or("Failed retrieving the query string"),
                line_wrap: true,
                xalign: 0.0
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
            gtk::Box {
                orientation: gtk::Orientation::Horizontal,
                gtk::Label {
                    label: &self.model.data.resultset_row_count.to_string(),
                    xalign: 0.0,
                    visible: !self.model.data.resultset_first_rows.is_empty()
                },
                gtk::Label {
                    label: " row(s)",
                    xalign: 0.0,
                    visible: !self.model.data.resultset_first_rows.is_empty()
                },
            },
            gtk::ScrolledWindow {
                #[name="resultset"]
                gtk::TreeView {
                    hexpand: true,
                    vexpand: true,
                    visible: !self.model.data.resultset_first_rows.is_empty()
                },
            }
            // gtk::Label {
            //     label: &self.model.data.resultset_first_rows
            //             .iter()
            //             .map(|r| r.join(", "))
            //             .collect::<Vec<_>>()
            //             .join("\n"),
            //     xalign: 0.0,
            // },
        }
    }
}
