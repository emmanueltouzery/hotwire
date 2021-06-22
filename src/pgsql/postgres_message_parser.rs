use super::postgres_details_widget;
use super::postgres_details_widget::PostgresCommEntry;
use crate::colors;
use crate::icons::Icon;
use crate::message_parser::{
    ClientServerInfo, MessageData, MessageInfo, MessageParser, StreamData, StreamGlobals,
};
use crate::pgsql::tshark_pgsql::{PostgresColType, PostgresWireMessage};
use crate::tshark_communication::{TSharkPacket, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use chrono::{NaiveDateTime, Utc};
use gtk::prelude::*;
use relm::ContainerWidget;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc;

#[cfg(test)]
use crate::tshark_communication::{parse_stream, parse_test_xml};
#[cfg(test)]
use chrono::NaiveDate;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkPacket) -> bool {
        msg.pgsql.is_some()
    }

    fn tshark_filter_string(&self) -> &'static str {
        "pgsql"
    }

    fn protocol_icon(&self) -> Icon {
        Icon::DATABASE
    }

    fn protocol_name(&self) -> &'static str {
        "PGSQL"
    }

    fn initial_globals(&self) -> StreamGlobals {
        StreamGlobals::Postgres(PostgresStreamGlobals::default())
    }

    fn add_to_stream(
        &self,
        mut stream: StreamData,
        new_packet: TSharkPacket,
    ) -> Result<StreamData, String> {
        let mut globals = stream.stream_globals.extract_postgres().unwrap();
        let timestamp = new_packet.basic_info.frame_time;
        if let Some(mds) = new_packet.pgsql {
            for md in mds {
                match md {
                    PostgresWireMessage::Startup {
                        username: Some(ref username),
                        database: Some(ref database),
                        application,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        stream
                            .messages
                            .push(MessageData::Postgres(PostgresMessageData {
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
                                resultset_datetime_cols: vec![],
                                resultset_col_types: vec![],
                            }));
                    }
                    PostgresWireMessage::Startup { .. } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                    }
                    PostgresWireMessage::Parse {
                        ref query,
                        ref statement,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        if let (Some(st), Some(q)) = (statement, query) {
                            globals.known_statements.insert((*st).clone(), (*q).clone());
                        }
                        globals.cur_query = query.clone();
                    }
                    PostgresWireMessage::Bind {
                        statement,
                        parameter_values,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        globals.was_bind = true;
                        globals.query_timestamp = Some(timestamp);
                        globals.cur_query_with_fallback = match (&globals.cur_query, &statement) {
                            (Some(_), _) => globals.cur_query.clone(),
                            (None, Some(s)) => Some(
                                globals
                                    .known_statements
                                    .get(s)
                                    .cloned()
                                    .unwrap_or(format!("Unknown statement: {}", s)),
                            ),
                            _ => None,
                        };
                        globals.cur_parameter_values = parameter_values.to_vec();
                    }
                    PostgresWireMessage::RowDescription {
                        col_names,
                        col_types,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        globals.cur_col_names = col_names;
                        globals.cur_col_types = col_types;
                        for col_type in &globals.cur_col_types {
                            match col_type {
                                PostgresColType::Bool => {
                                    globals.cur_rs_bool_cols.push(vec![]);
                                }
                                PostgresColType::Int2 | PostgresColType::Int4 => {
                                    globals.cur_rs_int_cols.push(vec![]);
                                }
                                PostgresColType::Timestamp => {
                                    globals.cur_rs_datetime_cols.push(vec![]);
                                }
                                PostgresColType::Int8 => {
                                    globals.cur_rs_bigint_cols.push(vec![]);
                                }
                                _ => {
                                    globals.cur_rs_string_cols.push(vec![]);
                                }
                            }
                        }
                    }
                    PostgresWireMessage::ResultSetRow { cols } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        globals.cur_rs_row_count += 1;
                        let mut int_col_idx = 0;
                        let mut datetime_col_idx = 0;
                        let mut bigint_col_idx = 0;
                        let mut bool_col_idx = 0;
                        let mut string_col_idx = 0;
                        if globals.cur_col_types.is_empty() {
                            // it's possible we don't have all the info about this query
                            // default to String for all the columns instead of dropping the data.
                            globals.cur_col_types = vec![PostgresColType::Text; cols.len()];
                            globals.cur_col_names = vec!["Col".to_string(); cols.len()];
                            for _ in &globals.cur_col_types {
                                globals.cur_rs_string_cols.push(vec![]);
                            }
                        };
                        for (col_type, val) in globals.cur_col_types.iter().zip(cols) {
                            match col_type {
                                PostgresColType::Bool => {
                                    globals.cur_rs_bool_cols[bool_col_idx].push(match &val[..] {
                                        "t" => Some(true),
                                        "null" => None,
                                        "f" => Some(false),
                                        _ => return Err(format!("expected bool value: {}", val)),
                                    });
                                    bool_col_idx += 1;
                                }
                                PostgresColType::Int2 | PostgresColType::Int4 => {
                                    globals.cur_rs_int_cols[int_col_idx].push(if val == "null" {
                                        None
                                    } else {
                                        let parsed: Option<i32> = val.parse().ok();
                                        if parsed.is_some() {
                                            parsed
                                        } else {
                                            return Err(format!("expected int value: {}", val));
                                        }
                                    });
                                    int_col_idx += 1;
                                }
                                PostgresColType::Timestamp => {
                                    globals.cur_rs_datetime_cols[datetime_col_idx].push(
                                        if val == "null" {
                                            None
                                        } else {
                                            let parsed = NaiveDateTime::parse_from_str(
                                                &val,
                                                "%Y-%m-%d %H:%M:%S%.f",
                                            )
                                            .ok();
                                            if parsed.is_some() {
                                                parsed
                                            } else {
                                                return Err(format!(
                                                    "expected datetime value: {}",
                                                    val
                                                ));
                                            }
                                        },
                                    );
                                    datetime_col_idx += 1;
                                }
                                PostgresColType::Int8 => {
                                    globals.cur_rs_bigint_cols[bigint_col_idx].push(
                                        if val == "null" {
                                            None
                                        } else {
                                            let parsed: Option<i64> = val.parse().ok();
                                            if parsed.is_some() {
                                                parsed
                                            } else {
                                                return Err(format!(
                                                    "expected int8 value: {}",
                                                    val
                                                ));
                                            }
                                        },
                                    );
                                    bigint_col_idx += 1;
                                }
                                _ => {
                                    globals.cur_rs_string_cols[string_col_idx]
                                        .push(Some(val).filter(|v| v != "null"));
                                    string_col_idx += 1;
                                }
                            }
                        }
                    }
                    PostgresWireMessage::ReadyForQuery => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        if globals.was_bind {
                            stream
                                .messages
                                .push(MessageData::Postgres(PostgresMessageData {
                                    query: globals.cur_query_with_fallback.map(Cow::Owned),
                                    query_timestamp: globals.query_timestamp.unwrap(), // know it was populated since was_bind is true
                                    result_timestamp: timestamp,
                                    parameter_values: globals.cur_parameter_values,
                                    resultset_col_names: globals.cur_col_names,
                                    resultset_row_count: globals.cur_rs_row_count,
                                    resultset_bool_cols: globals.cur_rs_bool_cols,
                                    resultset_string_cols: globals.cur_rs_string_cols,
                                    resultset_int_cols: globals.cur_rs_int_cols,
                                    resultset_bigint_cols: globals.cur_rs_bigint_cols,
                                    resultset_datetime_cols: globals.cur_rs_datetime_cols,
                                    resultset_col_types: globals.cur_col_types,
                                }));
                        }
                        globals.was_bind = false;
                        globals.cur_query_with_fallback = None;
                        globals.cur_query = None;
                        globals.cur_col_names = vec![];
                        globals.cur_parameter_values = vec![];
                        globals.cur_rs_row_count = 0;
                        globals.cur_rs_bool_cols = vec![];
                        globals.cur_rs_string_cols = vec![];
                        globals.cur_rs_int_cols = vec![];
                        globals.cur_rs_bigint_cols = vec![];
                        globals.cur_rs_datetime_cols = vec![];
                        globals.cur_col_types = vec![];
                        globals.query_timestamp = None;
                    }
                    PostgresWireMessage::CopyData => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        stream
                            .messages
                            .push(MessageData::Postgres(PostgresMessageData {
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
                                resultset_datetime_cols: vec![],
                                resultset_col_types: vec![],
                            }));
                    }
                }
            }
        }
        stream.stream_globals = StreamGlobals::Postgres(globals);
        Ok(stream)
    }

    fn finish_stream(&self, stream: StreamData) -> Result<StreamData, String> {
        Ok(stream)
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
        session_id: TcpStreamId,
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
                    &session_id.as_u32().to_value(),
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
                    &colors::STREAM_COLORS
                        [session_id.as_u32() as usize % colors::STREAM_COLORS.len()]
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

    fn matches_filter(&self, filter: &str, model: &gtk::TreeModel, iter: &gtk::TreeIter) -> bool {
        model
            .get_value(iter, 0)
            .get::<&str>()
            .unwrap()
            .unwrap()
            .to_lowercase()
            .contains(&filter.to_lowercase())
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
    ) -> Box<dyn Fn(mpsc::Sender<BgFunc>, MessageInfo)> {
        let component = Box::leak(Box::new(parent.add_widget::<PostgresCommEntry>((
            TcpStreamId(0),
            "0.0.0.0".parse().unwrap(),
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
                resultset_datetime_cols: vec![],
                resultset_col_types: vec![],
            },
            win_msg_sender,
            bg_sender,
        ))));
        Box::new(move |bg_sender, message_info| {
            component
                .stream()
                .emit(postgres_details_widget::Msg::DisplayDetails(
                    bg_sender,
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
    pub resultset_datetime_cols: Vec<Vec<Option<NaiveDateTime>>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PostgresStreamGlobals {
    known_statements: HashMap<String, String>,
    cur_query: Option<String>,
    cur_query_with_fallback: Option<String>,
    was_bind: bool,
    query_timestamp: Option<NaiveDateTime>,
    cur_rs_row_count: usize,
    cur_col_names: Vec<String>,
    cur_col_types: Vec<PostgresColType>,
    cur_parameter_values: Vec<String>,
    cur_rs_int_cols: Vec<Vec<Option<i32>>>,
    cur_rs_bigint_cols: Vec<Vec<Option<i64>>>,
    cur_rs_bool_cols: Vec<Vec<Option<bool>>>,
    cur_rs_string_cols: Vec<Vec<Option<String>>>,
    cur_rs_datetime_cols: Vec<Vec<Option<NaiveDateTime>>>,
}

#[test]
fn should_parse_simple_query() {
    let parsed = parse_stream(Postgres, parse_test_xml(
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
        .unwrap().messages;
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
        resultset_datetime_cols: vec![],
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
    let parsed = parse_stream(Postgres, parse_test_xml(
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
        .unwrap().messages;
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
            resultset_datetime_cols: vec![],
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
            resultset_datetime_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement_with_parameters() {
    let parsed = parse_stream(Postgres, parse_test_xml(
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
        .unwrap().messages;
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
            resultset_datetime_cols: vec![],
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
            resultset_datetime_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_not_generate_queries_for_just_a_ready_message() {
    let parsed = parse_stream(Postgres, parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .unwrap().messages;
    let expected: Vec<MessageData> = vec![];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_query_with_multiple_columns_and_nulls() {
    let parsed = parse_stream(Postgres, parse_test_xml(
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
        .unwrap().messages;
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
        resultset_datetime_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![vec![Some("26".to_string())]],
    })];
    assert_eq!(expected, parsed);
}

// this will happen if we don't catch the TCP stream at the beginning
#[test]
fn should_parse_query_with_no_parse_and_unknown_bind() {
    let parsed = parse_stream(Postgres, parse_test_xml(
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
        .unwrap().messages;
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
        resultset_datetime_cols: vec![],
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
