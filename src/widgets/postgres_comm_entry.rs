// https://www.postgresql.org/docs/12/protocol.html
use crate::icons::Icon;
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::MessageParser;
use crate::widgets::comm_remote_server::MessageParserDetailsMsg;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::{ContainerWidget, Widget};
use relm_derive::{widget, Msg};
use std::collections::HashMap;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.pgsql.is_some()
    }

    fn protocol_icon(&self) -> Icon {
        Icon::DATABASE
    }

    fn parse_stream(&self, stream: &Vec<TSharkCommunication>) -> Vec<MessageData> {
        let mut all_vals = vec![];
        for msg in stream {
            let root = msg.source.layers.pgsql.as_ref();
            match root {
                Some(serde_json::Value::Object(_)) => all_vals.push(root.unwrap()),
                Some(serde_json::Value::Array(vals)) => all_vals.extend(vals),
                _ => {}
            }
        }
        parse_pg_stream(all_vals)
    }

    fn prepare_treeview(&self, tv: &gtk::TreeView) -> gtk::ListStore {
        let liststore = gtk::ListStore::new(&[
            // TODO add: response time...
            String::static_type(), // query first line
            String::static_type(), // response info (number of rows..)
            u32::static_type(),    // stream_id
            u32::static_type(),    // index of the comm in the model vector
        ]);

        let query_col = gtk::TreeViewColumnBuilder::new()
            .title("Query")
            .expand(true)
            .fixed_width(100)
            .build();
        let cell_q_txt = gtk::CellRendererTextBuilder::new().build();
        query_col.pack_start(&cell_q_txt, true);
        query_col.add_attribute(&cell_q_txt, "text", 0);
        tv.append_column(&query_col);

        let result_col = gtk::TreeViewColumnBuilder::new()
            .title("Result")
            .fixed_width(100)
            .build();
        let cell_r_txt = gtk::CellRendererTextBuilder::new().build();
        result_col.pack_start(&cell_r_txt, true);
        result_col.add_attribute(&cell_r_txt, "text", 1);
        tv.append_column(&result_col);

        tv.set_model(Some(&liststore));

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
                    .unwrap_or("couldn't get query")
                    .to_value(),
            );
            ls.set_value(
                &iter,
                1,
                &format!("{} rows", postgres.resultset_row_count).to_value(),
            );
            ls.set_value(&iter, 2, &session_id.to_value());
            ls.set_value(&iter, 3, &(idx as i32).to_value());
        }
    }

    fn add_details_to_scroll(
        &self,
        parent: &gtk::ScrolledWindow,
    ) -> relm::StreamHandle<MessageParserDetailsMsg> {
        let component = Box::leak(Box::new(parent.add_widget::<PostgresCommEntry>(
            PostgresMessageData {
                query: None,
                parameter_values: vec![],
                resultset_col_names: vec![],
                resultset_row_count: 0,
                resultset_first_rows: vec![],
            },
        )));
        component.stream()
    }
}

#[derive(Debug)]
pub enum PostgresWireMessage {
    Parse {
        query: Option<String>,
        statement: Option<String>,
    },
    Bind {
        statement: Option<String>,
        parameter_values: Vec<String>,
    },
    RowDescription {
        col_names: Vec<String>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostgresMessageData {
    // for prepared queries, it's possible the declaration
    // occured before we started recording the stream.
    // in that case we won't be able to recover the query string.
    query: Option<String>,
    parameter_values: Vec<String>,
    resultset_col_names: Vec<String>,
    resultset_row_count: usize,
    resultset_first_rows: Vec<Vec<String>>,
}

fn parse_pg_stream(all_vals: Vec<&serde_json::Value>) -> Vec<MessageData> {
    let decoded_messages = all_vals.into_iter().filter_map(parse_pg_value).collect();
    merge_message_datas(decoded_messages)
}

// now postgres bound parameters.. $1, $2..
// for instance in session 34
fn parse_pg_value(pgsql: &serde_json::Value) -> Option<PostgresWireMessage> {
    let obj = pgsql.as_object();
    let typ = obj
        .and_then(|o| o.get("pgsql.type"))
        .and_then(|t| t.as_str());
    // if let Some(query_info) = obj.and_then(|o| o.get("pgsql.query")) {
    if typ == Some("Parse") {
        return Some(PostgresWireMessage::Parse {
            // query: format!("{} -> {}", time_relative, q),
            query: obj
                .and_then(|o| o.get("pgsql.query"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            statement: obj
                .and_then(|o| o.get("pgsql.statement"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
        });
    }
    if typ == Some("Bind") {
        // for prepared statements, the first time we get parse & statement+query
        // the following times we get bind and only statement (statement ID)
        // we can then recover the query from the statement id in post-processing.
        return Some(PostgresWireMessage::Bind {
            // query: format!("{} -> {}", time_relative, q),
            statement: obj
                .and_then(|o| o.get("pgsql.statement"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            parameter_values: obj
                .and_then(|o| o.iter().find(|(k, v)| k.starts_with("Parameter values: ")))
                .and_then(|(_k, v)| {
                    if let serde_json::Value::Object(o) = v {
                        if let Some(serde_json::Value::Array(vals)) = o.get("pgsql.val.data") {
                            return Some(
                                vals.iter()
                                    .filter_map(|v| v.as_str().and_then(hex_chars_to_string))
                                    .collect(),
                            );
                        }
                    }
                    None
                })
                .unwrap_or_else(|| vec![]),
        });
    }
    if typ == Some("Ready for query") {
        return Some(PostgresWireMessage::ReadyForQuery);
    }
    if typ == Some("Row description") {
        let tree = obj.unwrap().get("pgsql.field.count_tree").unwrap();
        let col_names = match tree.get("pgsql.col.name") {
            Some(serde_json::Value::String(s)) => {
                vec![s.to_string()]
            }
            Some(serde_json::Value::Array(ar)) => ar
                .into_iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect(),
            _ => vec![],
        };
        return Some(PostgresWireMessage::RowDescription { col_names });
    }
    if typ == Some("Data row") {
        let col_count = obj
            .unwrap()
            .get("pgsql.field.count")
            .unwrap()
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let tree = obj.unwrap().get("pgsql.field.count_tree").unwrap();
        let col_lengths: Vec<i32> = match tree.get("pgsql.val.length") {
            Some(serde_json::Value::String(s)) => {
                vec![s.parse().unwrap_or(0)]
            }
            Some(serde_json::Value::Array(ar)) => ar
                .into_iter()
                .map(|v| v.as_str().and_then(|s| s.parse().ok()).unwrap_or(0))
                .collect(),
            _ => vec![],
        };
        let mut raw_cols = match tree.get("pgsql.val.data") {
            Some(serde_json::Value::String(s)) => vec![hex_chars_to_string(s).unwrap_or_default()],
            Some(serde_json::Value::Array(ar)) => ar
                .into_iter()
                .map(|v| {
                    // format!(
                    //     "{} -> {}",
                    //     time_relative,
                    hex_chars_to_string(v.as_str().unwrap()).unwrap_or_default()
                    // )
                })
                .collect(),
            None => vec![],
            _ => panic!(),
        };
        raw_cols.reverse();
        // println!("col_lengths: {:?}", col_lengths);
        // dbg!(raw_cols.len());
        // dbg!(&raw_cols);
        let mut cols = vec![];
        for col_length in col_lengths {
            if col_length < 0 {
                cols.push("null".to_string());
            } else if col_length == 0 {
                cols.push("".to_string());
            } else {
                if let Some(val) = raw_cols.pop() {
                    cols.push(val);
                }
            }
        }
        if col_count != cols.len() {
            panic!("{} != {}", col_count, cols.len());
        }
        if !raw_cols.is_empty() {
            panic!("raw_cols: {:?}", raw_cols);
        }
        return Some(PostgresWireMessage::ResultSetRow { cols });
    }

    // "pgsql.type": "Bind",

    None
}

fn hex_chars_to_string(hex_chars: &str) -> Option<String> {
    let nocolons = hex_chars.replace(':', "");
    hex::decode(&nocolons)
        .ok()
        .map(|c| c.into_iter().collect())
        .and_then(|c: Vec<_>| {
            // the interpretation, null, digit or string is really guesswork...
            if c.first() == Some(&0u8) {
                // interpret as a number
                Some(i64::from_str_radix(&nocolons, 16).unwrap()) // i really want it to blow!
                    .map(|i| i.to_string())
            } else {
                String::from_utf8(c).ok()
            }
        })
    // hex_chars
    //     .split(':')
    //     .map(|s| u8::from_str_radix(s, 16))
    //     .join("")
}

fn merge_message_datas(mds: Vec<PostgresWireMessage>) -> Vec<MessageData> {
    let mut r = vec![];
    let mut cur_query = None;
    let mut cur_col_names = vec![];
    let mut cur_rs_row_count = 0;
    let mut cur_rs_first_rows = vec![];
    let mut known_statements = HashMap::new();
    let mut cur_query_with_fallback = None;
    let mut cur_parameter_values = vec![];
    let mut was_bind = false;
    for md in mds {
        match md {
            PostgresWireMessage::Parse {
                ref query,
                ref statement,
            } => {
                if let (Some(st), Some(q)) = (statement, query) {
                    known_statements.insert((*st).clone(), (*q).clone());
                }
                cur_query = query.clone();
            }
            PostgresWireMessage::Bind {
                statement,
                parameter_values,
            } => {
                was_bind = true;
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
            PostgresWireMessage::RowDescription { col_names } => {
                cur_col_names = col_names;
            }
            PostgresWireMessage::ResultSetRow { cols } => {
                cur_rs_row_count += 1;
                if cur_rs_row_count < 5 {
                    cur_rs_first_rows.push(cols);
                }
            }
            PostgresWireMessage::ReadyForQuery => {
                if was_bind {
                    r.push(MessageData::Postgres(PostgresMessageData {
                        query: cur_query_with_fallback,
                        parameter_values: cur_parameter_values,
                        resultset_col_names: cur_col_names,
                        resultset_row_count: cur_rs_row_count,
                        resultset_first_rows: cur_rs_first_rows,
                    }));
                }
                was_bind = false;
                cur_query_with_fallback = None;
                cur_query = None;
                cur_col_names = vec![];
                cur_parameter_values = vec![];
                cur_rs_row_count = 0;
                cur_rs_first_rows = vec![];
            }
        }
    }
    r
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
            .filter(|r| r.len() > 0) // list store can't have 0 columns
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
            MessageParserDetailsMsg::DisplayDetails(MessageData::Postgres(msg)) => {
                self.model.data = msg;

                let field_descs: Vec<_> = self
                    .model
                    .data
                    .resultset_first_rows
                    .first()
                    .filter(|r| r.len() > 0) // list store can't have 0 columns
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
            gtk::Separator {},
            gtk::Label {
                label: self.model.data.query.as_deref().unwrap_or("Failed retrieving the query string"),
                ellipsize: pango::EllipsizeMode::End,
                xalign: 0.0
            },
            gtk::Label {
                label: &self.model.data.parameter_values.join(", "),
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

#[cfg(test)]
fn as_json_array(json: &serde_json::Value) -> Vec<&serde_json::Value> {
    json.as_array().unwrap().iter().collect()
}

#[test]
fn should_parse_simple_query() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query: Some("select 1".to_string()),
        parameter_values: vec![],
        resultset_col_names: vec![],
        resultset_row_count: 2,
        resultset_first_rows: vec![
            vec!["PostgreSQL".to_string()],
            vec!["9.6.12 on x8".to_string()],
        ],
    })];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query: Some("select 1".to_string()),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_first_rows: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query: Some("select 1".to_string()),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 1,
            resultset_first_rows: vec![vec!["PostgreSQL".to_string()]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_prepared_statement_with_parameters() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
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
                  "pgsql.val.data": [
                        "54:52:55:45"
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query: Some("select $1".to_string()),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_first_rows: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query: Some("select $1".to_string()),
            parameter_values: vec!["TRUE".to_string()],
            resultset_col_names: vec![],
            resultset_row_count: 1,
            resultset_first_rows: vec![vec!["PostgreSQL".to_string()]],
        }),
    ];
    assert_eq!(expected, parsed);
}

#[test]
fn should_not_generate_queries_for_just_a_ready_message() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
            r#"
        [
          {
             "pgsql.type": "Ready for query"
          }
        ]
        "#,
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![];
    assert_eq!(expected, parsed);
}

#[test]
fn should_parse_query_with_multiple_columns_and_nulls() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
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
                  "pgsql.col.name": "version"
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query: Some("select 1".to_string()),
        parameter_values: vec![],
        resultset_col_names: vec!["version".to_string()],
        resultset_row_count: 1,
        resultset_first_rows: vec![vec![
            "Postg".to_string(),
            "null".to_string(),
            "".to_string(),
            "reSQL".to_string(),
        ]],
    })];
    assert_eq!(expected, parsed);
}

// this will happen if we don't catch the TCP stream at the beginning
#[test]
fn should_parse_query_with_no_parse_and_unknown_bind() {
    let parsed = parse_pg_stream(as_json_array(
        &serde_json::from_str(
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![MessageData::Postgres(PostgresMessageData {
        query: Some("Unknown statement: S_18".to_string()),
        parameter_values: vec![],
        resultset_col_names: vec![],
        resultset_row_count: 1,
        resultset_first_rows: vec![vec![
            "Postg".to_string(),
            "null".to_string(),
            "".to_string(),
            "reSQL".to_string(),
        ]],
    })];
    assert_eq!(expected, parsed);
}
