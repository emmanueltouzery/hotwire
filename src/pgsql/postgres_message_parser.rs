use super::postgres_details_widget;
use super::postgres_details_widget::PostgresCommEntry;
use crate::colors;
use crate::icons::Icon;
use crate::message_parser::{MessageInfo, MessageParser, StreamData};
use crate::pgsql::tshark_pgsql::{PostgresColType, PostgresWireMessage};
use crate::tshark_communication::TSharkPacket;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::win;
use crate::BgFunc;
use chrono::{NaiveDateTime, Utc};
use gtk::prelude::*;
use relm::ContainerWidget;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

#[cfg(test)]
use chrono::NaiveDate;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        msg.pgsql.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::DATABASE
    }

    fn parse_stream(&self, comms: Vec<TSharkPacket>) -> StreamData {
        let mut client_ip = comms.first().as_ref().unwrap().basic_info.ip_src.clone();
        let mut server_ip = comms.first().as_ref().unwrap().basic_info.ip_dst.clone();
        let mut server_port = comms.first().as_ref().unwrap().basic_info.port_dst;
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
            let timestamp = comm.basic_info.frame_time;
            if let Some(mds) = comm.pgsql {
                for md in mds {
                    match md {
                        PostgresWireMessage::Startup {
                            username: Some(ref username),
                            database: Some(ref database),
                            application,
                        } => {
                            if !set_correct_server_info {
                                server_ip = comm.basic_info.ip_src.to_string(); // TODO i think i can drop the to_string()
                                client_ip = comm.basic_info.ip_dst.to_string(); // TODO i think i can drop the to_string()
                                server_port = comm.basic_info.port_src;
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
                                server_ip = comm.basic_info.ip_dst.to_string();
                                client_ip = comm.basic_info.ip_src.to_string();
                                server_port = comm.basic_info.port_dst;
                                set_correct_server_info = true;
                            }
                        }
                        PostgresWireMessage::Parse {
                            ref query,
                            ref statement,
                        } => {
                            if !set_correct_server_info {
                                server_ip = comm.basic_info.ip_dst.to_string();
                                client_ip = comm.basic_info.ip_src.to_string();
                                server_port = comm.basic_info.port_dst;
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
                                server_ip = comm.basic_info.ip_dst.to_string();
                                client_ip = comm.basic_info.ip_src.to_string();
                                server_port = comm.basic_info.port_dst;
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
                                server_ip = comm.basic_info.ip_src.to_string();
                                client_ip = comm.basic_info.ip_dst.to_string();
                                server_port = comm.basic_info.port_src;
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
                                server_ip = comm.basic_info.ip_src.to_string();
                                client_ip = comm.basic_info.ip_dst.to_string();
                                server_port = comm.basic_info.port_src;
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
                                server_ip = comm.basic_info.ip_src.to_string();
                                client_ip = comm.basic_info.ip_dst.to_string();
                                server_port = comm.basic_info.port_src;
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
                                server_ip = comm.basic_info.ip_src.to_string();
                                client_ip = comm.basic_info.ip_dst.to_string();
                                server_port = comm.basic_info.port_src;
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
            client_ip: client_ip.clone(),
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

    fn requests_details_overlay(&self) -> bool {
        false
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
        _overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
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
            win_msg_sender,
            bg_sender,
        ))));
        Box::new(move |bg_sender, path, message_info| {
            component
                .stream()
                .emit(postgres_details_widget::Msg::DisplayDetails(
                    bg_sender,
                    path,
                    message_info,
                ))
        })
    }
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
fn parse_test_xml(xml: &str) -> Vec<TSharkPacket> {
    win::parse_pdml_stream(format!(test_fmt_str!(), xml).as_bytes()).unwrap()
}

#[test]
fn should_parse_simple_query() {
    let parsed = Postgres {}
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="25" pos="66">
    <field name="pgsql.type" showname="Type: Parse" size="1" pos="66" show="Parse" value="50"/>
    <field name="pgsql.query" show="select 1" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="116" pos="109">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="109" show="Data row" value="44"/>
    <field name="pgsql.field.count" showname="Field count: 1" size="2" pos="114" show="1" value="0001">
      <field name="pgsql.val.length" showname="Column length: 10" size="4" pos="116" show="10" value="00000069"/>
      <field name="pgsql.val.data" size="10" pos="120" show="50:6f:73:74:67:72:65:53:51:4c"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="116" pos="109">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="109" show="Data row" value="44"/>
    <field name="pgsql.field.count" showname="Field count: 1" size="2" pos="114" show="1" value="0001">
      <field name="pgsql.val.length" showname="Column length: 10" size="4" pos="116" show="10" value="00000069"/>
      <field name="pgsql.val.data" size="10" pos="120" show="39:2e:36:2e:31:32:20:6f:6e:20:78:38"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
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
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="25" pos="66">
    <field name="pgsql.type" showname="Type: Parse" size="1" pos="66" show="Parse" value="50"/>
    <field name="pgsql.query" show="select 1" />
    <field name="pgsql.statement" show="S_18"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
    <field name="pgsql.statement" show="S_18" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
    <field name="pgsql.statement" show="S_18" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="116" pos="109">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="109" show="Data row" value="44"/>
    <field name="pgsql.field.count" showname="Field count: 1" size="2" pos="114" show="1" value="0001">
      <field name="pgsql.val.length" showname="Column length: 10" size="4" pos="116" show="10" value="00000069"/>
      <field name="pgsql.val.data" size="10" pos="120" show="50:6f:73:74:67:72:65:53:51:4c"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
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
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
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
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="25" pos="66">
    <field name="pgsql.type" showname="Type: Parse" size="1" pos="66" show="Parse" value="50"/>
    <field name="pgsql.query" show="select $1" />
    <field name="pgsql.statement" show="S_18"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
    <field name="pgsql.statement" show="S_18" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
    <field name="pgsql.statement" show="S_18" />
    <field name="" show="Parameter formats: 3" size="2" pos="1193" value="0003">
      <field name="pgsql.format" showname="Format: Binary (1)" size="2" pos="1195" show="1" value="0001"/>
      <field name="pgsql.format" showname="Format: Binary (1)" size="2" pos="1197" show="1" value="0001"/>
      <field name="pgsql.format" showname="Format: Binary (1)" size="2" pos="1199" show="1" value="0001"/>
    </field>
    <field name="" show="Parameter values: 3" size="2" pos="1201" value="0003">
      <field name="pgsql.val.length" showname="Column length: 12" size="4" pos="1354" show="12" value="0000000c"/>
      <field name="pgsql.val.data" showname="Data: 303031343244413038394331" size="12" pos="1358" show="30:30:31:34:32:44:41:30:38:39:43:31" value="303031343244413038394331"/>
      <field name="pgsql.val.length" showname="Column length: -1" size="4" pos="1296" show="-1" value="ffffffff"/>
      <field name="pgsql.val.length" showname="Column length: 9" size="4" pos="1370" show="9" value="00000009"/>
      <field name="pgsql.val.data" showname="Data: 31302e382e302e3637" size="9" pos="1374" show="31:30:2e:38:2e:30:2e:36:37" value="31302e382e302e3637"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="116" pos="109">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="109" show="Data row" value="44"/>
    <field name="pgsql.field.count" showname="Field count: 1" size="2" pos="114" show="1" value="0001">
      <field name="pgsql.val.length" showname="Column length: 10" size="4" pos="116" show="10" value="00000069"/>
      <field name="pgsql.val.data" size="10" pos="120" show="50:6f:73:74:67:72:65:53:51:4c"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
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
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            query: Some(Cow::Borrowed("select $1")),
            parameter_values: vec![
                "00142DA089C1".to_string(),
                "null".to_string(),
                "10.8.0.67".to_string(),
            ],
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
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_query_with_multiple_columns_and_nulls() {
    let parsed = Postgres {}
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="25" pos="66">
    <field name="pgsql.type" showname="Type: Parse" size="1" pos="66" show="Parse" value="50"/>
    <field name="pgsql.query" show="select 1" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="33" pos="76">
    <field name="pgsql.type" showname="Type: Row description" size="1" pos="76" show="Row description" value="54"/>
    <field name="pgsql.length" showname="Length: 32" size="4" pos="77" show="32" value="00000020"/>
    <field name="pgsql.frontend" showname="Frontend: False" hide="yes" size="0" pos="76" show="0"/>
    <field name="pgsql.field.count" showname="Field count: 1" size="2" pos="81" show="1" value="0001">
      <field name="pgsql.col.name" showname="Column name: version" size="8" pos="83" show="version" value="76657273696f6e00">
        <field name="pgsql.oid.table" showname="Table OID: 0" size="4" pos="91" show="0" value="00000000"/>
        <field name="pgsql.col.index" showname="Column index: 0" size="2" pos="95" show="0" value="0000"/>
        <field name="pgsql.oid.type" showname="Type OID: 25" size="4" pos="97" show="25" value="00000019"/>
        <field name="pgsql.val.length" showname="Column length: -1" size="2" pos="101" show="-1" value="ffff"/>
        <field name="pgsql.col.typemod" showname="Type modifier: -1" size="4" pos="103" show="-1" value="ffffffff"/>
        <field name="pgsql.format" showname="Format: Text (0)" size="2" pos="107" show="0" value="0000"/>
      </field>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="67" pos="87">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="87" show="Data row" value="44"/>
    <field name="pgsql.length" showname="Length: 66" size="4" pos="88" show="66" value="00000042"/>
    <field name="pgsql.frontend" showname="Frontend: False" hide="yes" size="0" pos="87" show="0"/>
    <field name="pgsql.field.count" showname="Field count: 4" size="2" pos="92" show="4" value="0004">
      <field name="pgsql.val.length" showname="Column length: 4" size="4" pos="94" show="4" value="00000004"/>
      <field name="pgsql.val.data" showname="Data: 0000001a" size="4" pos="98" show="00:00:00:1a" value="0000001a"/>
      <field name="pgsql.val.length" showname="Column length: -1" size="4" pos="2255" show="-1" value="ffffffff"/>
      <field name="pgsql.val.length" showname="Column length: 7" size="4" pos="102" show="7" value="00000007"/>
      <field name="pgsql.val.data" showname="Data: 47454e4552414c" size="7" pos="106" show="47:45:4e:45:52:41:4c" value="47454e4552414c"/>
      <field name="pgsql.val.length" showname="Column length: 20" size="4" pos="113" show="20" value="00000014"/>
      <field name="pgsql.val.data" showname="Data: 4150504c49434154494f4e5f54494d455a4f4e45" size="20" pos="117" show="41:50:50:4c:49:43:41:54:49:4f:4e:5f:54:49:4d:45:5a:4f:4e:45" value="4150504c49434154494f4e5f54494d455a4f4e45"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        query: Some(Cow::Borrowed("select 1")),
        parameter_values: vec![],
        resultset_col_names: vec!["version".to_string()],
        resultset_row_count: 1,
        resultset_col_types: vec![PostgresColType::Text],
        resultset_int_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![vec![Some("26".to_string())]],
    })];
    assert_eq!(expected, parsed);
}

// this will happen if we don't catch the TCP stream at the beginning
#[test]
fn should_parse_query_with_no_parse_and_unknown_bind() {
    let parsed = Postgres {}
        .parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="25" pos="66">
    <field name="pgsql.type" showname="Type: Parse" size="1" pos="66" show="Parse" value="50"/>
    <field name="pgsql.query" show="select 1" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="13" pos="91">
    <field name="pgsql.type" showname="Type: Bind" size="1" pos="91" show="Bind" value="42"/>
    <field name="pgsql.statement" show="S_18" />
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="67" pos="87">
    <field name="pgsql.type" showname="Type: Data row" size="1" pos="87" show="Data row" value="44"/>
    <field name="pgsql.length" showname="Length: 66" size="4" pos="88" show="66" value="00000042"/>
    <field name="pgsql.frontend" showname="Frontend: False" hide="yes" size="0" pos="87" show="0"/>
    <field name="pgsql.field.count" showname="Field count: 4" size="2" pos="92" show="4" value="0004">
      <field name="pgsql.val.length" showname="Column length: 4" size="4" pos="94" show="4" value="00000004"/>
      <field name="pgsql.val.data" showname="Data: 0000001a" size="4" pos="98" show="00:00:00:1a" value="0000001a"/>
      <field name="pgsql.val.length" showname="Column length: -1" size="4" pos="2255" show="-1" value="ffffffff"/>
      <field name="pgsql.val.length" showname="Column length: 7" size="4" pos="102" show="7" value="00000007"/>
      <field name="pgsql.val.data" showname="Data: 47454e4552414c" size="7" pos="106" show="47:45:4e:45:52:41:4c" value="47454e4552414c"/>
      <field name="pgsql.val.length" showname="Column length: 20" size="4" pos="113" show="20" value="00000014"/>
      <field name="pgsql.val.data" showname="Data: 4150504c49434154494f4e5f54494d455a4f4e45" size="20" pos="117" show="41:50:50:4c:49:43:41:54:49:4f:4e:5f:54:49:4d:45:5a:4f:4e:45" value="4150504c49434154494f4e5f54494d455a4f4e45"/>
    </field>
  </proto>
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .messages;
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
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
            vec![Some("26".to_string())],
            vec![None],
            vec![Some("GENERAL".to_string())],
            vec![Some("APPLICATION_TIMEZONE".to_string())],
        ],
    })];
    assert_eq!(expected, parsed);
}
