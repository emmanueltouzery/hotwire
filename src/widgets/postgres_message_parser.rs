use super::comm_info_header;
use super::comm_info_header::CommInfoHeader;
use super::comm_remote_server::MessageData;
use super::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::colors;
use crate::icons::Icon;
use crate::pgsql::tshark_pgsql::{PostgresColType, PostgresWireMessage};
use crate::tshark_communication::{TSharkIpLayer, TSharkIpV6Layer};
use crate::BgFunc;
use crate::TSharkCommunication;
use chrono::{NaiveDateTime, Utc};
use gtk::prelude::*;
use itertools::Itertools;
use regex::Regex;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

#[cfg(test)]
use crate::pgsql::tshark_pgsql::TSharkPgsql;
#[cfg(test)]
use crate::tshark_communication::{TSharkFrameLayer, TSharkLayers, TSharkSource, TSharkTcpLayer};
#[cfg(test)]
use chrono::NaiveDate;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.pgsql.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::DATABASE
    }

    fn parse_stream(&self, comms: Vec<TSharkCommunication>) -> StreamData {
        let mut server_ip = comms
            .first()
            .as_ref()
            .unwrap()
            .source
            .layers
            .ip_dst()
            .clone();
        let mut server_port = comms
            .first()
            .as_ref()
            .unwrap()
            .source
            .layers
            .tcp
            .as_ref()
            .unwrap()
            .port_dst;
        let mut messages = vec![];
        let mut cur_query = None;
        let mut cur_col_names = vec![];
        let mut cur_col_types = vec![];
        let mut cur_rs_row_count = 0;
        let mut cur_rs_int_cols: Vec<Vec<Option<i32>>> = vec![];
        let mut cur_rs_bigint_cols: Vec<Vec<Option<i64>>> = vec![];
        let mut cur_rs_bool_cols: Vec<Vec<Option<bool>>> = vec![];
        let mut cur_rs_string_cols: Vec<Vec<Option<String>>> = vec![];
        let mut known_statements = HashMap::new();
        let mut cur_query_with_fallback = None;
        let mut was_bind = false;
        let mut cur_parameter_values = vec![];
        let mut query_timestamp = None;
        let mut set_correct_server_info = false;
        for comm in comms {
            let timestamp = comm.source.layers.frame.frame_time;
            if let Some(pgsql) = comm.source.layers.pgsql {
                let mds = pgsql.messages;
                for md in mds {
                    match md {
                        PostgresWireMessage::Startup {
                            username: Some(ref username),
                            database: Some(ref database),
                            application,
                        } => {
                            if !set_correct_server_info {
                                server_ip = dst_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_dst;
                                set_correct_server_info = true;
                            }
                            messages.push(MessageData::Postgres(PostgresMessageData {
                                query: Some(Cow::Owned(format!(
                                    "LOGIN: user: {}, db: {}, app: {}",
                                    username,
                                    database,
                                    application.as_deref().unwrap_or("-")
                                ))),
                                query_timestamp: timestamp,
                                result_timestamp: timestamp,
                                parameter_values: vec![],
                                resultset_col_names: vec![],
                                resultset_row_count: 0,
                                resultset_int_cols: vec![],
                                resultset_bigint_cols: vec![],
                                resultset_bool_cols: vec![],
                                resultset_string_cols: vec![],
                                resultset_col_types: vec![],
                            }));
                        }
                        PostgresWireMessage::Startup { .. } => {
                            if !set_correct_server_info {
                                server_ip = dst_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_dst;
                                set_correct_server_info = true;
                            }
                        }
                        PostgresWireMessage::Parse {
                            ref query,
                            ref statement,
                        } => {
                            if !set_correct_server_info {
                                server_ip = dst_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_dst;
                                set_correct_server_info = true;
                            }
                            if let (Some(st), Some(q)) = (statement, query) {
                                known_statements.insert((*st).clone(), (*q).clone());
                            }
                            cur_query = query.clone();
                        }
                        PostgresWireMessage::Bind {
                            statement,
                            parameter_values,
                        } => {
                            if !set_correct_server_info {
                                server_ip = dst_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_dst;
                                set_correct_server_info = true;
                            }
                            was_bind = true;
                            query_timestamp = Some(timestamp);
                            cur_query_with_fallback = match (&cur_query, &statement) {
                                (Some(_), _) => cur_query.clone(),
                                (None, Some(s)) => Some(
                                    known_statements
                                        .get(s)
                                        .cloned()
                                        .unwrap_or(format!("Unknown statement: {}", s)),
                                ),
                                _ => None,
                            };
                            cur_parameter_values = parameter_values.to_vec();
                        }
                        PostgresWireMessage::RowDescription {
                            col_names,
                            col_types,
                        } => {
                            if !set_correct_server_info {
                                server_ip = src_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_src;
                                set_correct_server_info = true;
                            }
                            cur_col_names = col_names;
                            cur_col_types = col_types;
                            for col_type in &cur_col_types {
                                match col_type {
                                    PostgresColType::Bool => {
                                        cur_rs_bool_cols.push(vec![]);
                                    }
                                    PostgresColType::Int2 | PostgresColType::Int4 => {
                                        cur_rs_int_cols.push(vec![]);
                                    }
                                    PostgresColType::Int8 => {
                                        cur_rs_bigint_cols.push(vec![]);
                                    }
                                    _ => {
                                        cur_rs_string_cols.push(vec![]);
                                    }
                                }
                            }
                        }
                        PostgresWireMessage::ResultSetRow { cols } => {
                            if !set_correct_server_info {
                                server_ip = src_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_src;
                                set_correct_server_info = true;
                            }
                            cur_rs_row_count += 1;
                            let mut int_col_idx = 0;
                            let mut bigint_col_idx = 0;
                            let mut bool_col_idx = 0;
                            let mut string_col_idx = 0;
                            if cur_col_types.is_empty() {
                                // it's possible we don't have all the info about this query
                                // default to String for all the columns instead of dropping the data.
                                cur_col_types = vec![PostgresColType::Text; cols.len()];
                                cur_col_names = vec!["Col".to_string(); cols.len()];
                                for _ in &cur_col_types {
                                    cur_rs_string_cols.push(vec![]);
                                }
                            };
                            for (col_type, val) in cur_col_types.iter().zip(cols) {
                                match col_type {
                                    PostgresColType::Bool => {
                                        cur_rs_bool_cols[bool_col_idx].push(match &val[..] {
                                            "t" => Some(true),
                                            "null" => None,
                                            "f" => Some(false),
                                            _ => panic!("unexpected bool value: {}", val),
                                        });
                                        bool_col_idx += 1;
                                    }
                                    PostgresColType::Int2 | PostgresColType::Int4 => {
                                        cur_rs_int_cols[int_col_idx].push(if val == "null" {
                                            None
                                        } else {
                                            let parsed: Option<i32> = val.parse().ok();
                                            if parsed.is_some() {
                                                parsed
                                            } else {
                                                panic!("unexpected int value: {}", val);
                                            }
                                        });
                                        int_col_idx += 1;
                                    }
                                    // PostgresColType::Timestamp => {

                                    // }
                                    PostgresColType::Int8 => {
                                        cur_rs_bigint_cols[bigint_col_idx].push(if val == "null" {
                                            None
                                        } else {
                                            let parsed: Option<i64> = val.parse().ok();
                                            if let Some(p) = parsed {
                                                parsed
                                            } else {
                                                panic!("unexpected int8 value: {}", val);
                                            }
                                        });
                                        bigint_col_idx += 1;
                                    }
                                    _ => {
                                        cur_rs_string_cols[string_col_idx]
                                            .push(Some(val).filter(|v| v != "null"));
                                        string_col_idx += 1;
                                    }
                                }
                            }
                        }
                        PostgresWireMessage::ReadyForQuery => {
                            if !set_correct_server_info {
                                server_ip = src_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_src;
                                set_correct_server_info = true;
                            }
                            if was_bind {
                                messages.push(MessageData::Postgres(PostgresMessageData {
                                    query: cur_query_with_fallback.map(Cow::Owned),
                                    query_timestamp: query_timestamp.unwrap(), // know it was populated since was_bind is true
                                    result_timestamp: timestamp,
                                    parameter_values: cur_parameter_values,
                                    resultset_col_names: cur_col_names,
                                    resultset_row_count: cur_rs_row_count,
                                    resultset_bool_cols: cur_rs_bool_cols,
                                    resultset_string_cols: cur_rs_string_cols,
                                    resultset_int_cols: cur_rs_int_cols,
                                    resultset_bigint_cols: cur_rs_bigint_cols,
                                    resultset_col_types: cur_col_types,
                                }));
                            }
                            was_bind = false;
                            cur_query_with_fallback = None;
                            cur_query = None;
                            cur_col_names = vec![];
                            cur_parameter_values = vec![];
                            cur_rs_row_count = 0;
                            cur_rs_bool_cols = vec![];
                            cur_rs_string_cols = vec![];
                            cur_rs_int_cols = vec![];
                            cur_rs_bigint_cols = vec![];
                            cur_col_types = vec![];
                            query_timestamp = None;
                        }
                        PostgresWireMessage::CopyData => {
                            if !set_correct_server_info {
                                server_ip = src_ip(
                                    comm.source.layers.ip.as_ref(),
                                    comm.source.layers.ipv6.as_ref(),
                                )
                                .to_string();
                                server_port = comm.source.layers.tcp.as_ref().unwrap().port_src;
                                set_correct_server_info = true;
                            }
                            messages.push(MessageData::Postgres(PostgresMessageData {
                                query: Some(Cow::Borrowed("COPY DATA")),
                                query_timestamp: timestamp,
                                result_timestamp: timestamp,
                                parameter_values: vec![],
                                resultset_col_names: vec![],
                                resultset_row_count: 0,
                                resultset_int_cols: vec![],
                                resultset_bigint_cols: vec![],
                                resultset_bool_cols: vec![],
                                resultset_string_cols: vec![],
                                resultset_col_types: vec![],
                            }));
                        }
                    }
                }
            }
        }
        StreamData {
            server_ip: server_ip.clone(),
            server_port,
            messages,
            summary_details: None,
        }
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        let streamcolor_col = gtk::TreeViewColumnBuilder::new()
            .title("S")
            .fixed_width(10)
            .sort_column_id(2)
            .build();
        let cell_s_txt = gtk::CellRendererTextBuilder::new().build();
        streamcolor_col.pack_start(&cell_s_txt, true);
        streamcolor_col.add_attribute(&cell_s_txt, "background", 10);
        tv.append_column(&streamcolor_col);

        let queryt_col = gtk::TreeViewColumnBuilder::new()
            .title("Type")
            .fixed_width(24)
            .sort_column_id(9)
            .build();
        let cell_qt_txt = gtk::CellRendererPixbufBuilder::new().build();
        queryt_col.pack_start(&cell_qt_txt, true);
        queryt_col.add_attribute(&cell_qt_txt, "icon-name", 9);
        tv.append_column(&queryt_col);

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
            .sort_column_id(0)
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
            .sort_column_id(8)
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
    }

    fn get_empty_liststore(&self) -> gtk::ListStore {
        gtk::ListStore::new(&[
            String::static_type(), // query first line
            String::static_type(), // response info (number of rows..)
            u32::static_type(),    // stream_id
            u32::static_type(),    // index of the comm in the model vector
            String::static_type(), // query start timestamp (string)
            i64::static_type(),    // query start timestamp (integer, for sorting)
            i32::static_type(),    // query duration (nanos, for sorting)
            String::static_type(), // query duration display
            i64::static_type(),    // number of rows, for sorting
            String::static_type(), // query type: update, insert..
            String::static_type(), // stream color
        ])
    }

    fn populate_treeview(
        &self,
        ls: &gtk::ListStore,
        session_id: u32,
        messages: &[MessageData],
        start_idx: i32,
    ) {
        // println!("adding {} rows", messages.len());
        for (idx, message) in messages.iter().enumerate() {
            let postgres = message.as_postgres().unwrap();
            ls.insert_with_values(
                None,
                &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
                &[
                    &postgres
                        .query
                        .as_deref()
                        .map(|q| if q.len() > 250 { &q[..250] } else { q })
                        .unwrap_or("couldn't get query")
                        .replace("\n", "")
                        .to_value(),
                    &format!("{} rows", postgres.resultset_row_count).to_value(),
                    &session_id.to_value(),
                    &(start_idx + idx as i32).to_value(),
                    &postgres.query_timestamp.to_string().to_value(),
                    &postgres.query_timestamp.timestamp_nanos().to_value(),
                    &(postgres.result_timestamp - postgres.query_timestamp)
                        .num_milliseconds()
                        .to_value(),
                    &format!(
                        "{} ms",
                        (postgres.result_timestamp - postgres.query_timestamp).num_milliseconds()
                    )
                    .to_value(),
                    &(postgres.resultset_row_count as u32).to_value(),
                    &get_query_type_desc(&postgres.query).to_value(),
                    &colors::STREAM_COLORS[session_id as usize % colors::STREAM_COLORS.len()]
                        .to_value(),
                ],
            );
        }
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        let model_sort = gtk::TreeModelSort::new(ls);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        _bg_sender: mpsc::Sender<BgFunc>,
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, PathBuf, MessageInfo)> {
        let component = Box::leak(Box::new(parent.add_widget::<PostgresCommEntry>((
            0,
            "".to_string(),
            PostgresMessageData {
                query: None,
                query_timestamp: Utc::now().naive_local(),
                result_timestamp: Utc::now().naive_local(),
                parameter_values: vec![],
                resultset_col_names: vec![],
                resultset_row_count: 0,
                resultset_int_cols: vec![],
                resultset_bigint_cols: vec![],
                resultset_bool_cols: vec![],
                resultset_string_cols: vec![],
                resultset_col_types: vec![],
            },
        ))));
        Box::new(move |bg_sender, path, message_info| {
            component
                .stream()
                .emit(Msg::DisplayDetails(bg_sender, path, message_info))
        })
    }
}

fn src_ip<'a>(ip: Option<&'a TSharkIpLayer>, ipv6: Option<&'a TSharkIpV6Layer>) -> &'a str {
    ip.map(|i| &i.ip_src)
        .unwrap_or_else(|| &ipv6.unwrap().ip_src)
}

fn dst_ip<'a>(ip: Option<&'a TSharkIpLayer>, ipv6: Option<&'a TSharkIpV6Layer>) -> &'a str {
    ip.map(|i| &i.ip_dst)
        .unwrap_or_else(|| &ipv6.unwrap().ip_dst)
}

fn get_query_type_desc(query: &Option<Cow<'static, str>>) -> &'static str {
    if query.as_ref().filter(|q| q.len() >= 5).is_none() {
        "-"
    } else {
        let start_lower = query.as_ref().unwrap()[0..5].to_ascii_lowercase();
        if start_lower.starts_with("inser") {
            "insert"
        } else if start_lower.starts_with("selec") {
            "select"
        } else if start_lower.starts_with("updat") {
            "update"
        } else if start_lower.starts_with("delet") {
            "delete"
        } else if start_lower.starts_with("commi") {
            "commit"
        } else if start_lower.starts_with("rollb") {
            "rollback"
        } else if start_lower.starts_with("set ") {
            "system"
        } else if start_lower.starts_with("drop ") {
            "drop"
        } else if start_lower.starts_with("creat") {
            "create"
        } else if start_lower.starts_with("alter") {
            "alter"
        } else if start_lower.starts_with("do ") {
            "plsql"
        } else if start_lower.starts_with("login") {
            "login"
        } else if start_lower.starts_with("copy ") {
            // copy data
            "copy"
        } else if start_lower.starts_with("begin") {
            "bookmark"
        } else {
            "other"
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostgresMessageData {
    // for prepared queries, it's possible the declaration
    // occured before we started recording the stream.
    // in that case we won't be able to recover the query string.
    pub query_timestamp: NaiveDateTime,
    pub result_timestamp: NaiveDateTime,
    pub query: Option<Cow<'static, str>>,
    pub parameter_values: Vec<String>,
    pub resultset_col_names: Vec<String>,
    pub resultset_row_count: usize,
    pub resultset_col_types: Vec<PostgresColType>,
    pub resultset_string_cols: Vec<Vec<Option<String>>>,
    pub resultset_bool_cols: Vec<Vec<Option<bool>>>,
    pub resultset_int_cols: Vec<Vec<Option<i32>>>,
    pub resultset_bigint_cols: Vec<Vec<Option<i64>>>,
}

pub struct Model {
    stream_id: u32,
    client_ip: String,
    data: PostgresMessageData,
    list_store: Option<gtk::ListStore>,
    syntax_highlight: Vec<(Regex, String)>,
}

#[derive(Msg, Debug)]
pub enum Msg {
    DisplayDetails(mpsc::Sender<BgFunc>, PathBuf, MessageInfo),
}

#[widget]
impl Widget for PostgresCommEntry {
    fn init_view(&mut self) {}

    fn model(_relm: &relm::Relm<Self>, params: (u32, String, PostgresMessageData)) -> Model {
        let (stream_id, client_ip, data) = params;
        Model {
            data,
            stream_id,
            client_ip,
            list_store: None,
            syntax_highlight: Self::prepare_syntax_highlight(),
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
                _path,
                MessageInfo {
                    stream_id,
                    client_ip,
                    message_data: MessageData::Postgres(msg),
                },
            ) => {
                self.model.data = msg;
                self.streams
                    .comm_info_header
                    .emit(comm_info_header::Msg::Update(client_ip.clone(), stream_id));
                self.model.stream_id = stream_id;
                self.model.client_ip = client_ip;

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
                    let mut bool_idx = 0;
                    let mut int_idx = 0;
                    let mut bigint_idx = 0;
                    let mut str_idx = 0;
                    for (col_idx, col_type) in
                        self.model.data.resultset_col_types.iter().enumerate()
                    {
                        match col_type {
                            PostgresColType::Bool => {
                                list_store.set_value(
                                    &iter,
                                    col_idx as u32,
                                    &self.model.data.resultset_bool_cols[bool_idx][row_idx]
                                        .map(|v| Cow::Owned(v.to_string()))
                                        .unwrap_or(Cow::Borrowed("null"))
                                        .to_value(),
                                );
                                bool_idx += 1;
                            }
                            PostgresColType::Int2 | PostgresColType::Int4 => {
                                list_store.set_value(
                                    &iter,
                                    col_idx as u32,
                                    &self.model.data.resultset_int_cols[int_idx][row_idx]
                                        .map(|v| Cow::Owned(v.to_string()))
                                        .unwrap_or(Cow::Borrowed("null"))
                                        .to_value(),
                                );
                                int_idx += 1;
                            }
                            // PostgresColType::Int8 | PostgresColType::Timestamp => {
                            PostgresColType::Int8 => {
                                list_store.set_value(
                                    &iter,
                                    col_idx as u32,
                                    &self.model.data.resultset_int_cols[bigint_idx][row_idx]
                                        .map(|v| Cow::Owned(v.to_string()))
                                        .unwrap_or(Cow::Borrowed("null"))
                                        .to_value(),
                                );
                                bigint_idx += 1;
                            }
                            _ => {
                                list_store.set_value(
                                    &iter,
                                    col_idx as u32,
                                    &self.model.data.resultset_string_cols[str_idx][row_idx]
                                        .as_deref()
                                        .unwrap_or("null")
                                        .to_value(),
                                );
                                str_idx += 1;
                            }
                        }
                    }
                }
                self.widgets.resultset.set_model(Some(&list_store));
                self.model.list_store = Some(list_store);
            }
            _ => {}
        }
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
                        gtk::Label {
                            label: &self.model.data.resultset_row_count.to_string(),
                            xalign: 0.0,
                            visible: self.model.data.resultset_row_count > 0
                        },
                        gtk::Label {
                            label: " row(s)",
                            xalign: 0.0,
                            visible: !self.model.data.resultset_row_count > 0
                        },
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

#[cfg(test)]
fn as_json_array(json: &str) -> Vec<TSharkCommunication> {
    let items: Vec<TSharkPgsql> = serde_json::de::from_str(json).unwrap();
    items
        .into_iter()
        .map(|p| TSharkCommunication {
            source: TSharkSource {
                layers: TSharkLayers {
                    frame: TSharkFrameLayer {
                        frame_time: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
                    },
                    ip: Some(TSharkIpLayer {
                        ip_src: "127.0.0.1".to_string(),
                        ip_dst: "127.0.0.1".to_string(),
                    }),
                    ipv6: None,
                    tcp: Some(TSharkTcpLayer {
                        seq_number: 0,
                        stream: 0,
                        port_src: 0,
                        port_dst: 0,
                    }),
                    http: None,
                    pgsql: Some(p),
                    tls: None,
                },
            },
        })
        .collect()
}

#[test]
fn should_parse_simple_query() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Parse",
             "pgsql.query": "select 1"
          },
          {
             "pgsql.type": "Bind"
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "1",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": "10",
                 "pgsql.val.data": "50:6f:73:74:67:72:65:53:51:4c"
             }
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "1",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": "10",
                 "pgsql.val.data": "39:2e:36:2e:31:32:20:6f:6e:20:78:38"
             }
          },
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        query: Some(Cow::Borrowed("select 1")),
        parameter_values: vec![],
        resultset_col_names: vec!["Col".to_string()],
        resultset_row_count: 2,
        resultset_col_types: vec![PostgresColType::Text],
        resultset_int_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![vec![
            Some("PostgreSQL".to_string()),
            Some("9.6.12 on x8".to_string()),
        ]],
    })];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Parse",
             "pgsql.query": "select 1",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Bind",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Ready for query"
          },
          {
             "pgsql.type": "Bind",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "1",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": "10",
                 "pgsql.val.data": "50:6f:73:74:67:72:65:53:51:4c"
             }
          },
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some(Cow::Borrowed("select 1")),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_col_types: vec![],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some(Cow::Borrowed("select 1")),
            parameter_values: vec![],
            resultset_col_names: vec!["Col".to_string()],
            resultset_row_count: 1,
            resultset_col_types: vec![PostgresColType::Text],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement_with_parameters() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Parse",
             "pgsql.query": "select $1",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Bind",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Ready for query"
          },
          {
             "pgsql.type": "Bind",
             "pgsql.statement": "S_18",
             "Parameter values: 1": {
                  "pgsql.val.length": [
                        "4",
                        "-1",
                        "5"
                  ],
                  "pgsql.val.data": [
                        "54:52:55:45",
                        "54:52:55:45:52"
                  ]
             }
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "1",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": "10",
                 "pgsql.val.data": "50:6f:73:74:67:72:65:53:51:4c"
             }
          },
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some(Cow::Borrowed("select $1")),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_col_types: vec![],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some(Cow::Borrowed("select $1")),
            parameter_values: vec!["TRUE".to_string(), "null".to_string(), "TRUER".to_string()],
            resultset_col_names: vec!["Col".to_string()],
            resultset_row_count: 1,
            resultset_col_types: vec![PostgresColType::Text],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_not_generate_queries_for_just_a_ready_message() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_query_with_multiple_columns_and_nulls() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Parse",
             "pgsql.query": "select 1"
          },
          {
             "pgsql.type": "Bind"
          },
          {
             "pgsql.type": "Row description",
             "pgsql.field.count_tree": {
                  "pgsql.col.name": ["version", "col2", "col3", "col4"],
                  "pgsql.col.name_tree": [
                      {"pgsql.oid.type": "19"},
                      {"pgsql.oid.type": "19"},
                      {"pgsql.oid.type": "19"},
                      {"pgsql.oid.type": "19"}
                  ]
              }
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "4",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": ["5", "-1", "0", "5"],
                 "pgsql.val.data": ["50:6f:73:74:67", "72:65:53:51:4c"]
             }
          },
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        query: Some(Cow::Borrowed("select 1")),
        parameter_values: vec![],
        resultset_col_names: vec![
            "version".to_string(),
            "col2".to_string(),
            "col3".to_string(),
            "col4".to_string(),
        ],
        resultset_row_count: 1,
        resultset_col_types: vec![
            PostgresColType::Name,
            PostgresColType::Name,
            PostgresColType::Name,
            PostgresColType::Name,
        ],
        resultset_int_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![
            vec![Some("Postg".to_string())],
            vec![None],
            vec![Some("".to_string())],
            vec![Some("reSQL".to_string())],
        ],
    })];
    assert_eq!(expected, parsed);
}

// this will happen if we don't catch the TCP stream at the beginning
#[test]
fn should_parse_query_with_no_parse_and_unknown_bind() {
    let parsed = Postgres {}
        .parse_stream(as_json_array(
            r#"
        [
          {
             "pgsql.type": "Parse",
             "pgsql.query": "select 1"
          },
          {
             "pgsql.type": "Ready for query"
          },
          {
             "pgsql.type": "Bind",
             "pgsql.statement": "S_18"
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "4",
             "pgsql.field.count_tree": {
                 "pgsql.val.length": ["5", "-1", "0", "5"],
                 "pgsql.val.data": ["50:6f:73:74:67", "72:65:53:51:4c"]
             }
          },
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        query: Some(Cow::Borrowed("Unknown statement: S_18")),
        parameter_values: vec![],
        resultset_col_names: vec![
            "Col".to_string(),
            "Col".to_string(),
            "Col".to_string(),
            "Col".to_string(),
        ],
        resultset_row_count: 1,
        resultset_col_types: vec![
            PostgresColType::Text,
            PostgresColType::Text,
            PostgresColType::Text,
            PostgresColType::Text,
        ],
        resultset_int_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![
            vec![Some("Postg".to_string())],
            vec![None],
            vec![Some("".to_string())],
            vec![Some("reSQL".to_string())],
        ],
    })];
    assert_eq!(expected, parsed);
}
