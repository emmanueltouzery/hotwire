use super::postgres_details_widget;
use super::postgres_details_widget::PostgresCommEntry;
use crate::colors;
use crate::custom_streams_store;
use crate::custom_streams_store::{ClientServerInfo, CustomStreamsStore};
use crate::icons::Icon;
use crate::pgsql::tshark_pgsql::{PostgresColType, PostgresWireMessage};
use crate::search_expr;
use crate::tshark_communication::{TSharkPacket, TcpStreamId};
use crate::widgets::win;
use crate::BgFunc;
use chrono::{NaiveDateTime, Utc};
use gtk::prelude::*;
use relm::ContainerWidget;
use std::borrow::Cow;
use std::collections::HashMap;
use std::iter;
use std::str;
use std::str::FromStr;
use std::sync::mpsc;
use strum::VariantNames;
use strum_macros::{EnumString, EnumVariantNames};

#[cfg(test)]
use {
    crate::custom_streams_store::common_tests_parse_stream,
    crate::tshark_communication::parse_test_xml, chrono::NaiveDate,
};

#[derive(Default)]
pub struct PostgresStreamData {
    pub stream_globals: PostgresStreamGlobals,
    pub client_server: Option<ClientServerInfo>,
    pub messages: Vec<PostgresMessageData>,
    pub summary_details: Option<String>,
}

#[derive(Default)]
pub struct PostgresStreamsStore {
    streams: HashMap<TcpStreamId, PostgresStreamData>,
    component: Option<relm::Component<PostgresCommEntry>>,
}

impl PostgresStreamsStore {
    fn get_msg_info(
        &self,
        stream_id: TcpStreamId,
        msg_idx: usize,
    ) -> Option<(&PostgresMessageData, ClientServerInfo)> {
        let stream = self.streams.get(&stream_id)?;
        let msg = stream.messages.get(msg_idx)?;
        Some((msg, stream.client_server?))
    }
}

#[derive(EnumString, EnumVariantNames)]
enum PostgresFilterKeys {
    #[strum(serialize = "pg.query")]
    QueryString,
    #[strum(serialize = "pg.resultset")]
    ResultSet,
    #[strum(serialize = "pg.query_param")]
    QueryParamValue,
}

fn get_pg_message<'a, 'b>(
    streams: &'a HashMap<TcpStreamId, PostgresStreamData>,
    model: &'b gtk::TreeModel,
    iter: &'b gtk::TreeIter,
) -> Option<&'a PostgresMessageData> {
    let (stream_id, idx) = custom_streams_store::get_message_helper(model, iter);
    streams
        .get(&stream_id)
        .and_then(|s| s.messages.get(idx as usize))
}

impl CustomStreamsStore for PostgresStreamsStore {
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

    fn tcp_stream_ids(&self) -> Vec<TcpStreamId> {
        self.streams.keys().copied().collect()
    }

    fn has_stream_id(&self, stream_id: TcpStreamId) -> bool {
        self.streams.contains_key(&stream_id)
    }

    fn reset(&mut self) {
        self.streams = HashMap::new();
    }

    fn stream_message_count(&self, stream_id: TcpStreamId) -> Option<usize> {
        self.streams.get(&stream_id).map(|s| s.messages.len())
    }

    fn stream_summary_details(&self, stream_id: TcpStreamId) -> Option<&str> {
        self.streams
            .get(&stream_id)
            .and_then(|s| s.summary_details.as_deref())
    }

    fn stream_client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo> {
        self.streams.get(&stream_id).and_then(|s| s.client_server)
    }

    fn is_empty(&self) -> bool {
        self.streams.is_empty()
    }

    fn add_to_stream(
        &mut self,
        stream_id: TcpStreamId,
        new_packet: TSharkPacket,
    ) -> Result<Option<ClientServerInfo>, String> {
        let stream = self
            .streams
            .entry(stream_id)
            .or_insert_with(PostgresStreamData::default);
        let timestamp = new_packet.basic_info.frame_time;
        if let Some(mds) = new_packet.pgsql {
            for md in mds {
                match md {
                    PostgresWireMessage::Startup {
                        username: Some(ref username),
                        database: Some(ref database),
                        application,
                    } => {
                        match stream.summary_details.as_ref() {
                            None => {
                                stream.summary_details = Some(database.to_string());
                            }
                            Some(other_db) if !other_db.contains(database) => {
                                stream.summary_details =
                                    Some(format!("{}, {}", other_db, database));
                            }
                            _ => {}
                        }
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        stream.messages.push(PostgresMessageData {
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
                        });
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
                        ref param_types,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        if let (Some(st), Some(q)) = (statement, query) {
                            stream
                                .stream_globals
                                .known_statements
                                .insert((*st).clone(), (*q).clone());
                        }
                        stream.stream_globals.cur_query = query.clone();
                        stream.stream_globals.parse_param_types = param_types.clone();
                    }
                    PostgresWireMessage::Bind {
                        statement,
                        parameter_lengths_and_vals,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_dst,
                                client_ip: new_packet.basic_info.ip_src,
                                server_port: new_packet.basic_info.port_dst,
                            });
                        }
                        stream.stream_globals.was_bind = true;
                        stream.stream_globals.query_timestamp = Some(timestamp);
                        stream.stream_globals.cur_query_with_fallback =
                            match (&stream.stream_globals.cur_query, &statement) {
                                (Some(_), _) => stream.stream_globals.cur_query.clone(),
                                (None, Some(s)) => Some(
                                    stream
                                        .stream_globals
                                        .known_statements
                                        .get(s)
                                        .cloned()
                                        .unwrap_or(format!("Unknown statement: {}", s)),
                                ),
                                _ => None,
                            };
                        stream.stream_globals.cur_parameter_values = parameter_lengths_and_vals
                            .iter()
                            .zip(
                                stream
                                    .stream_globals
                                    .parse_param_types
                                    .iter()
                                    .chain(iter::repeat(&PostgresColType::Unknown)),
                            )
                            .map(|((length, val), typ)| (*typ, decode_param(*typ, val, *length)))
                            .collect();
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
                        stream.stream_globals.cur_col_names = col_names;
                        stream.stream_globals.cur_col_types = col_types;
                        for col_type in &stream.stream_globals.cur_col_types {
                            match col_type {
                                PostgresColType::Bool => {
                                    stream.stream_globals.cur_rs_bool_cols.push(vec![]);
                                }
                                PostgresColType::Int2 | PostgresColType::Int4 => {
                                    stream.stream_globals.cur_rs_int_cols.push(vec![]);
                                }
                                PostgresColType::Timestamp => {
                                    stream.stream_globals.cur_rs_datetime_cols.push(vec![]);
                                }
                                PostgresColType::Int8 => {
                                    stream.stream_globals.cur_rs_bigint_cols.push(vec![]);
                                }
                                _ => {
                                    stream.stream_globals.cur_rs_string_cols.push(vec![]);
                                }
                            }
                        }
                    }
                    PostgresWireMessage::ResultSetRow {
                        col_lengths_and_vals,
                    } => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        handle_pgsql_resultset_row(
                            &mut stream.stream_globals,
                            col_lengths_and_vals
                                .iter()
                                .map(|(_l, v)| v.clone())
                                .collect(),
                        )?;
                    }
                    PostgresWireMessage::ReadyForQuery => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }

                        // reset all the globals, but keep known_statements
                        let globals = std::mem::take(&mut stream.stream_globals);
                        stream.stream_globals.known_statements = globals.known_statements;
                        if globals.was_bind {
                            stream.messages.push(PostgresMessageData {
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
                            });
                        }
                    }
                    PostgresWireMessage::CopyData => {
                        if stream.client_server.is_none() {
                            stream.client_server = Some(ClientServerInfo {
                                server_ip: new_packet.basic_info.ip_src,
                                client_ip: new_packet.basic_info.ip_dst,
                                server_port: new_packet.basic_info.port_src,
                            });
                        }
                        stream.messages.push(PostgresMessageData {
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
                        });
                    }
                }
            }
        }
        Ok(stream.client_server)
    }

    fn finish_stream(&mut self, _stream_id: TcpStreamId) -> Result<(), String> {
        Ok(())
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) {
        let streamcolor_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("S")
            .fixed_width(10)
            .sort_column_id(2)
            .build();
        let cell_s_txt = gtk::builders::CellRendererTextBuilder::new().build();
        streamcolor_col.pack_start(&cell_s_txt, true);
        streamcolor_col.add_attribute(&cell_s_txt, "background", 10);
        tv.append_column(&streamcolor_col);

        let queryt_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("Type")
            .fixed_width(24)
            .sort_column_id(9)
            .build();
        let cell_qt_txt = gtk::builders::CellRendererPixbufBuilder::new().build();
        queryt_col.pack_start(&cell_qt_txt, true);
        queryt_col.add_attribute(&cell_qt_txt, "icon-name", 9);
        tv.append_column(&queryt_col);

        let timestamp_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("Timestamp")
            .resizable(true)
            .sort_column_id(5)
            .build();
        let cell_t_txt = gtk::builders::CellRendererTextBuilder::new().build();
        timestamp_col.pack_start(&cell_t_txt, true);
        timestamp_col.add_attribute(&cell_t_txt, "text", 4);
        tv.append_column(&timestamp_col);

        let query_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("Query")
            .expand(true)
            .resizable(true)
            .sort_column_id(0)
            .build();
        let cell_q_txt = gtk::builders::CellRendererTextBuilder::new()
            .ellipsize(pango::EllipsizeMode::End)
            .build();
        query_col.pack_start(&cell_q_txt, true);
        query_col.add_attribute(&cell_q_txt, "text", 0);
        tv.append_column(&query_col);

        let result_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("Result")
            .resizable(true)
            .sort_column_id(8)
            .build();
        let cell_r_txt = gtk::builders::CellRendererTextBuilder::new().build();
        result_col.pack_start(&cell_r_txt, true);
        result_col.add_attribute(&cell_r_txt, "text", 1);
        tv.append_column(&result_col);

        let duration_col = gtk::builders::TreeViewColumnBuilder::new()
            .title("Duration")
            .resizable(true)
            .sort_column_id(6)
            .build();
        let cell_d_txt = gtk::builders::CellRendererTextBuilder::new().build();
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
        start_idx: usize,
        item_count: usize,
    ) {
        let messages = &self.streams.get(&session_id).unwrap().messages;
        // println!("adding {} rows", messages.len());
        for (idx, postgres) in messages.iter().skip(start_idx).take(item_count).enumerate() {
            ls.insert_with_values(
                None,
                &[
                    (
                        0,
                        &postgres
                            .query
                            .as_deref()
                            .map(|q| if q.len() > 250 { &q[..250] } else { q })
                            .unwrap_or("couldn't get query")
                            .replace("\n", "")
                            .to_value(),
                    ),
                    (
                        1,
                        &format!("{} rows", postgres.resultset_row_count).to_value(),
                    ),
                    (
                        custom_streams_store::TREE_STORE_STREAM_ID_COL_IDX,
                        &session_id.as_u32().to_value(),
                    ),
                    (
                        custom_streams_store::TREE_STORE_MESSAGE_INDEX_COL_IDX,
                        &((start_idx + idx) as i32).to_value(),
                    ),
                    (4, &postgres.query_timestamp.to_string().to_value()),
                    (5, &postgres.query_timestamp.timestamp_nanos().to_value()),
                    (
                        6,
                        &(postgres.result_timestamp - postgres.query_timestamp)
                            .num_milliseconds()
                            .to_value(),
                    ),
                    (
                        7,
                        &format!(
                            "{} ms",
                            (postgres.result_timestamp - postgres.query_timestamp)
                                .num_milliseconds()
                        )
                        .to_value(),
                    ),
                    (8, &(postgres.resultset_row_count as u32).to_value()),
                    (9, &get_query_type_desc(&postgres.query).to_value()),
                    (
                        10,
                        &colors::STREAM_COLORS
                            [session_id.as_u32() as usize % colors::STREAM_COLORS.len()]
                        .to_value(),
                    ),
                ],
            );
        }
    }

    fn end_populate_treeview(&self, tv: &gtk::TreeView, ls: &gtk::ListStore) {
        let model_sort = gtk::TreeModelSort::new(ls);
        model_sort.set_sort_column_id(gtk::SortColumn::Index(5), gtk::SortType::Ascending);
        tv.set_model(Some(&model_sort));
    }

    fn supported_filter_keys(&self) -> &'static [&'static str] {
        PostgresFilterKeys::VARIANTS
    }

    fn matches_filter(
        &self,
        filter: &search_expr::SearchOpExpr,
        model: &gtk::TreeModel,
        iter: &gtk::TreeIter,
    ) -> bool {
        let streams = &self.streams;
        if let Ok(filter_key) = PostgresFilterKeys::from_str(filter.filter_key) {
            match filter_key {
                PostgresFilterKeys::QueryString => model
                    .value(iter, 0)
                    .get::<&str>()
                    .unwrap()
                    .to_lowercase()
                    .contains(&filter.filter_val.to_lowercase()),
                PostgresFilterKeys::ResultSet => {
                    let fv = filter.filter_val.to_lowercase();
                    get_pg_message(streams, model, iter).map_or(false, |pg_msg| {
                        pg_msg.resultset_string_cols.iter().any(|v| {
                            v.iter().any(|c| {
                                c.as_ref().map_or(false, |v| v.to_lowercase().contains(&fv))
                            })
                        })
                    })
                }
                PostgresFilterKeys::QueryParamValue => {
                    let fv = filter.filter_val.to_lowercase();
                    get_pg_message(streams, model, iter).map_or(false, |pg_msg| {
                        pg_msg
                            .parameter_values
                            .iter()
                            .any(|(_type, v)| v.to_lowercase().contains(&fv))
                    })
                }
            }
        } else {
            true
        }
    }

    fn requests_details_overlay(&self) -> bool {
        false
    }

    fn add_details_to_scroll(
        &mut self,
        parent: &gtk::ScrolledWindow,
        _overlay: Option<&gtk::Overlay>,
        bg_sender: mpsc::Sender<BgFunc>,
        win_msg_sender: relm::StreamHandle<win::Msg>,
    ) {
        let component = parent.add_widget::<PostgresCommEntry>((
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
        ));
        self.component = Some(component);
    }

    fn display_in_details_widget(
        &self,
        bg_sender: mpsc::Sender<BgFunc>,
        stream_id: TcpStreamId,
        msg_idx: usize,
    ) {
        if let Some((pg_msg, client_server)) = self.get_msg_info(stream_id, msg_idx) {
            self.component.as_ref().unwrap().stream().emit(
                postgres_details_widget::Msg::DisplayDetails(
                    bg_sender,
                    client_server.client_ip,
                    stream_id,
                    pg_msg.clone(),
                ),
            )
        }
    }
}

fn decode_bool(val: &str) -> Option<bool> {
    match hex_chars_to_string(val).as_deref() {
        Some("t") => Some(true),
        Some("1") => Some(true),
        Some("null") => None,
        Some("f") => Some(false),
        Some("0") => Some(false),
        _ => {
            // eprintln!("expected bool value: {}", val,);
            Some(true) // I've seen values such as 54525545 or 46414c5345.. maybe PG optimises "anything non-zero is true"?
        }
    }
}

fn decode_unknown(val: &str) -> String {
    // try to detect string or binary/int, because
    // the latter will contain \0
    // maybe i rather should check:
    //       <field name="pgsql.format" showname="Format: Binary (1)" size="2" pos="106" show="1" value="0001"/>
    //       <field name="pgsql.format" showname="Format: Text (0)" size="2" pos="108" show="0" value="0000"/>
    // which I think is always present
    if hex_chars_to_bytes(val)
        .unwrap_or_else(Vec::new)
        .iter()
        .any(|b| b == &0)
    {
        decode_integer_as_str::<i32>(PostgresColType::Int8, val)
    } else {
        hex_chars_to_string(val).unwrap_or_else(|| val.to_string())
    }
}

fn decode_param(typ: PostgresColType, val: &str, length: i64) -> String {
    if val == "null" || length == -1 {
        return "null".to_string();
    }
    match typ {
        PostgresColType::ByteArray => format!("Byte array ({} bytes): {}", length, val),
        PostgresColType::Bool => match decode_bool(val) {
            Some(true) => "t",
            Some(false) => "f",
            None => "null",
        }
        .to_string(),
        PostgresColType::Int4 | PostgresColType::Int2 => decode_integer_as_str::<i32>(typ, val),
        PostgresColType::Int8 => decode_integer_as_str::<i64>(typ, val),
        PostgresColType::Unknown => decode_unknown(val),
        _ => hex_chars_to_string(val).unwrap_or_else(|| format!("Error decoding: {}", val)),
    }
}

fn decode_integer<T: num_traits::Num + ToString + FromStr>(
    _typ: PostgresColType,
    val: &str,
) -> Option<T> {
    if val.starts_with("00") {
        T::from_str_radix(val, 16).ok()
    } else {
        hex_chars_to_bytes(val)
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| s.parse().ok())
    }
}

// I was unable to find a logic in the PDML. Sometimes I would get hex-encoded
// digits, sometimes the raw values. It seemed more often for int4 the hex-encoded
// and for int8 the raw values but not always...
// Would have to look in wireshark's source but... another time...
// I don't know whether the type matters. Putting it in the type sig so I can assert
// on known values
fn decode_integer_as_str<T: num_traits::Num + ToString + FromStr>(
    typ: PostgresColType,
    val: &str,
) -> String {
    decode_integer(typ, val)
        .map(|i: T| i.to_string())
        .unwrap_or_else(|| val.to_string())
}

fn handle_pgsql_resultset_row(
    globals: &mut PostgresStreamGlobals,
    cols: Vec<String>,
) -> Result<(), String> {
    globals.cur_rs_row_count += 1;
    let mut int_col_idx = 0;
    let mut datetime_col_idx = 0;
    let mut bigint_col_idx = 0;
    let mut bool_col_idx = 0;
    let mut string_col_idx = 0;
    if globals.cur_col_types.is_empty() {
        // it's possible we don't have all the info about this query
        // default to String for all the columns instead of dropping the data.
        globals.cur_col_types = vec![PostgresColType::Unknown; cols.len()];
        globals.cur_col_names = vec!["Col".to_string(); cols.len()];
        for _ in &globals.cur_col_types {
            globals.cur_rs_string_cols.push(vec![]);
        }
    };
    for (col_type, val) in globals.cur_col_types.iter().zip(cols) {
        match col_type {
            PostgresColType::Bool => {
                globals.cur_rs_bool_cols[bool_col_idx].push(decode_bool(&val));
                bool_col_idx += 1;
            }
            PostgresColType::Int2 | PostgresColType::Int4 => {
                let unhexed = hex_chars_to_string(&val);
                globals.cur_rs_int_cols[int_col_idx].push(if unhexed.as_deref() == Some("null") {
                    None
                } else {
                    Some(decode_integer::<i32>(*col_type, &val).unwrap_or(0))
                });
                int_col_idx += 1;
            }
            PostgresColType::Timestamp => {
                let val_str = hex_chars_to_string(&val);
                globals.cur_rs_datetime_cols[datetime_col_idx].push(
                    if val_str.as_deref() == Some("null") {
                        None
                    } else {
                        let parsed = val_str.and_then(|v| {
                            NaiveDateTime::parse_from_str(&v, "%Y-%m-%d %H:%M:%S%.f").ok()
                        });
                        if parsed.is_some() {
                            parsed
                        } else {
                            return Err(format!("expected datetime value: {}", val));
                        }
                    },
                );
                datetime_col_idx += 1;
            }
            PostgresColType::Int8 => {
                let unhexed = hex_chars_to_string(&val);
                globals.cur_rs_bigint_cols[bigint_col_idx].push(
                    if unhexed.as_deref() == Some("null") {
                        None
                    } else {
                        let parsed = unhexed.and_then(|h| h.parse::<i64>().ok());
                        if parsed.is_some() {
                            parsed
                        } else {
                            return Err(format!("expected int8 value: {}", val));
                        }
                    },
                );
                bigint_col_idx += 1;
            }
            PostgresColType::Unknown => {
                globals.cur_rs_string_cols[string_col_idx].push(
                    if hex_chars_to_string(&val).as_deref() == Some("null") {
                        None
                    } else {
                        Some(decode_unknown(&val))
                    },
                );
                string_col_idx += 1;
            }
            _ => {
                globals.cur_rs_string_cols[string_col_idx]
                    .push(hex_chars_to_string(&val).filter(|v| v != "null"));
                string_col_idx += 1;
            }
        }
    }
    Ok(())
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

fn hex_chars_to_string(hex_chars: &str) -> Option<String> {
    hex_chars_to_bytes(hex_chars)
        .as_ref()
        .and_then(|b| str::from_utf8(b).ok())
        .map(|s| s.to_string())
}

fn hex_chars_to_bytes(hex_chars: &str) -> Option<Vec<u8>> {
    let nocolons = hex_chars.replace(':', "");
    hex::decode(&nocolons).ok().map(|c| c.into_iter().collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostgresMessageData {
    // for prepared queries, it's possible the declaration
    // occured before we started recording the stream.
    // in that case we won't be able to recover the query string.
    pub query_timestamp: NaiveDateTime,
    pub result_timestamp: NaiveDateTime,
    pub query: Option<Cow<'static, str>>,
    pub parameter_values: Vec<(PostgresColType, String)>,
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
    parse_param_types: Vec<PostgresColType>,
    cur_query_with_fallback: Option<String>,
    was_bind: bool,
    query_timestamp: Option<NaiveDateTime>,
    cur_rs_row_count: usize,
    cur_col_names: Vec<String>,
    cur_col_types: Vec<PostgresColType>,
    cur_parameter_values: Vec<(PostgresColType, String)>,
    cur_rs_int_cols: Vec<Vec<Option<i32>>>,
    cur_rs_bigint_cols: Vec<Vec<Option<i64>>>,
    cur_rs_bool_cols: Vec<Vec<Option<bool>>>,
    cur_rs_string_cols: Vec<Vec<Option<String>>>,
    cur_rs_datetime_cols: Vec<Vec<Option<NaiveDateTime>>>,
}

#[cfg(test)]
fn tests_parse_stream(
    packets: Result<Vec<TSharkPacket>, String>,
) -> Result<Vec<PostgresMessageData>, String> {
    let mut parser = PostgresStreamsStore::default();
    let sid = common_tests_parse_stream(&mut parser, packets)?;
    Ok(parser.streams.get(&sid).unwrap().messages.clone())
}

#[test]
fn should_parse_simple_query() {
    let parsed = tests_parse_stream(parse_test_xml(
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
        .unwrap();
    let expected = vec![PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        query: Some(Cow::Borrowed("select 1")),
        parameter_values: vec![],
        resultset_col_names: vec!["Col".to_string()],
        resultset_row_count: 2,
        resultset_col_types: vec![PostgresColType::Unknown],
        resultset_int_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_datetime_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![vec![
            Some("PostgreSQL".to_string()),
            Some("9.6.12 on x8".to_string()),
        ]],
    }];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement() {
    let parsed = tests_parse_stream(parse_test_xml(
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
        .unwrap();
    let expected = vec![
        PostgresMessageData {
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
        },
        PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            query: Some(Cow::Borrowed("select 1")),
            parameter_values: vec![],
            resultset_col_names: vec!["Col".to_string()],
            resultset_row_count: 1,
            resultset_col_types: vec![PostgresColType::Unknown],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_datetime_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        },
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement_with_parameters() {
    let parsed = tests_parse_stream(parse_test_xml(
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
        .unwrap();
    let expected = vec![
        PostgresMessageData {
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
        },
        PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
            query: Some(Cow::Borrowed("select $1")),
            parameter_values: vec![
                (PostgresColType::Unknown, "00142DA089C1".to_string()),
                (PostgresColType::Unknown, "null".to_string()),
                (PostgresColType::Unknown, "10.8.0.67".to_string()),
            ],
            resultset_col_names: vec!["Col".to_string()],
            resultset_row_count: 1,
            resultset_col_types: vec![PostgresColType::Unknown],
            resultset_int_cols: vec![],
            resultset_bigint_cols: vec![],
            resultset_datetime_cols: vec![],
            resultset_bool_cols: vec![],
            resultset_string_cols: vec![vec![Some("PostgreSQL".to_string())]],
        },
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_not_generate_queries_for_just_a_ready_message() {
    let parsed = tests_parse_stream(parse_test_xml(
            r#"
  <proto name="pgsql" showname="PostgreSQL" size="6" pos="239">
    <field name="pgsql.type" showname="Type: Ready for query" size="1" pos="239" show="Ready for query" value="5a"/>
  </proto>
        "#,
        ))
        .unwrap();
    let expected: Vec<PostgresMessageData> = vec![];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_query_with_multiple_columns_and_nulls() {
    let parsed = tests_parse_stream(parse_test_xml(
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
        <field name="pgsql.oid.type" showname="Type OID: 23" size="4" pos="97" show="23" value="00000017"/>
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
        .unwrap();
    let expected = vec![PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        query: Some(Cow::Borrowed("select 1")),
        parameter_values: vec![],
        resultset_col_names: vec!["version".to_string()],
        resultset_row_count: 1,
        resultset_col_types: vec![PostgresColType::Int4],
        resultset_string_cols: vec![],
        resultset_bigint_cols: vec![],
        resultset_datetime_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_int_cols: vec![vec![Some(26)]],
    }];
    assert_eq!(expected, parsed);
}

// this will happen if we don't catch the TCP stream at the beginning
#[test]
fn should_parse_query_with_no_parse_and_unknown_bind() {
    let parsed = tests_parse_stream(parse_test_xml(
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
  <proto name="pgsql" showname="PostgreSQL" size="33" pos="76">
    <field name="pgsql.type" showname="Type: Row description" size="1" pos="76" show="Row description" value="54"/>
    <field name="pgsql.length" showname="Length: 32" size="4" pos="77" show="32" value="00000020"/>
    <field name="pgsql.frontend" showname="Frontend: False" hide="yes" size="0" pos="76" show="0"/>
    <field name="pgsql.field.count" showname="Field count: 4" size="2" pos="81" show="1" value="0001">
      <field name="pgsql.col.name" showname="Column name: version" size="8" pos="83" show="version" value="76657273696f6e00">
        <field name="pgsql.oid.table" showname="Table OID: 0" size="4" pos="91" show="0" value="00000000"/>
        <field name="pgsql.col.index" showname="Column index: 0" size="2" pos="95" show="0" value="0000"/>
        <field name="pgsql.oid.type" showname="Type OID: 23" size="4" pos="97" show="23" value="00000017"/>
        <field name="pgsql.val.length" showname="Column length: -1" size="2" pos="101" show="-1" value="ffff"/>
        <field name="pgsql.col.typemod" showname="Type modifier: -1" size="4" pos="103" show="-1" value="ffffffff"/>
        <field name="pgsql.format" showname="Format: Text (0)" size="2" pos="107" show="0" value="0000"/>
      </field>
      <field name="pgsql.col.name" showname="Column name: version" size="8" pos="83" show="version" value="76657273696f6e00">
        <field name="pgsql.oid.table" showname="Table OID: 0" size="4" pos="91" show="0" value="00000000"/>
        <field name="pgsql.col.index" showname="Column index: 0" size="2" pos="95" show="0" value="0000"/>
        <field name="pgsql.oid.type" showname="Type OID: 25" size="4" pos="97" show="25" value="00000019"/>
        <field name="pgsql.val.length" showname="Column length: -1" size="2" pos="101" show="-1" value="ffff"/>
        <field name="pgsql.col.typemod" showname="Type modifier: -1" size="4" pos="103" show="-1" value="ffffffff"/>
        <field name="pgsql.format" showname="Format: Text (0)" size="2" pos="107" show="0" value="0000"/>
      </field>
      <field name="pgsql.col.name" showname="Column name: version" size="8" pos="83" show="version" value="76657273696f6e00">
        <field name="pgsql.oid.table" showname="Table OID: 0" size="4" pos="91" show="0" value="00000000"/>
        <field name="pgsql.col.index" showname="Column index: 0" size="2" pos="95" show="0" value="0000"/>
        <field name="pgsql.oid.type" showname="Type OID: 25" size="4" pos="97" show="25" value="00000019"/>
        <field name="pgsql.val.length" showname="Column length: -1" size="2" pos="101" show="-1" value="ffff"/>
        <field name="pgsql.col.typemod" showname="Type modifier: -1" size="4" pos="103" show="-1" value="ffffffff"/>
        <field name="pgsql.format" showname="Format: Text (0)" size="2" pos="107" show="0" value="0000"/>
      </field>
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
        .unwrap();
    let expected = vec![PostgresMessageData {
        query_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 5).and_hms_nano(8, 49, 52, 736275000),
        query: Some(Cow::Borrowed("Unknown statement: S_18")),
        parameter_values: vec![],
        resultset_col_names: vec![
            "version".to_string(),
            "version".to_string(),
            "version".to_string(),
            "version".to_string(),
        ],
        resultset_row_count: 1,
        resultset_col_types: vec![
            PostgresColType::Int4,
            PostgresColType::Text,
            PostgresColType::Text,
            PostgresColType::Text,
        ],
        resultset_int_cols: vec![vec![Some(26)]],
        resultset_bigint_cols: vec![],
        resultset_datetime_cols: vec![],
        resultset_bool_cols: vec![],
        resultset_string_cols: vec![
            vec![None],
            vec![Some("GENERAL".to_string())],
            vec![Some("APPLICATION_TIMEZONE".to_string())],
        ],
    }];
    assert_eq!(expected, parsed);
}

#[test]
fn decode_23() {
    // known from SELECT typname FROM pg_catalog.pg_type WHERE oid=23
    assert_eq!(
        "23",
        decode_integer_as_str::<i32>(PostgresColType::Int4, "3233")
    );
}
