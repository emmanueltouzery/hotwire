// https://www.postgresql.org/docs/12/protocol.html
use crate::widgets::comm_remote_server::MessageData;
use crate::widgets::comm_remote_server::MessageParser;
use crate::TSharkCommunication;
use gtk::prelude::*;
use relm::Widget;
use relm_derive::{widget, Msg};
use std::collections::HashMap;

pub struct Postgres;

impl MessageParser for Postgres {
    fn is_my_message(&self, msg: &TSharkCommunication) -> bool {
        msg.source.layers.pgsql.is_some()
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
}

pub enum PostgresWireMessage {
    Parse {
        query: Option<String>,
        statement: Option<String>,
    },
    Bind {
        statement: Option<String>,
        parameter_values: Vec<String>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostgresMessageData {
    Query {
        // for prepared queries, it's possible the declaration
        // occured before we started recording the stream.
        // in that case we won't be able to recover the query string.
        query: Option<String>,
        parameter_values: Vec<String>,
    },
    ResultSet {
        row_count: usize,
        first_rows: Vec<Vec<String>>,
    },
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
    if typ == Some("Data row") {
        let col_count = obj
            .unwrap()
            .get("pgsql.field.count")
            .unwrap()
            .as_str()
            .unwrap()
            .parse::<i32>()
            .unwrap();
        let tree = obj.unwrap().get("pgsql.field.count_tree").unwrap();
        let cols = match tree.get("pgsql.val.data") {
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
            if c.iter().all(|x| x == &0u8) {
                Some("null".to_string())
            } else if c.first() == Some(&0u8) {
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
    let mut cur_rs_row_count = 0;
    let mut cur_rs_first_rows = vec![];
    let mut known_statements = HashMap::new();
    for md in mds {
        match md {
            PostgresWireMessage::Parse {
                ref query,
                ref statement,
            } => {
                if cur_rs_row_count > 0 {
                    r.push(MessageData::Postgres(PostgresMessageData::ResultSet {
                        row_count: cur_rs_row_count,
                        first_rows: cur_rs_first_rows,
                    }));
                    cur_rs_row_count = 0;
                    cur_rs_first_rows = vec![];
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
                let query_with_fallback = match (&cur_query, &statement) {
                    (Some(_), _) => cur_query.clone(),
                    (None, Some(s)) => known_statements.get(s).cloned(),
                    _ => None,
                };
                r.push(MessageData::Postgres(PostgresMessageData::Query {
                    query: query_with_fallback,
                    parameter_values: parameter_values.to_vec(),
                }));
            }
            PostgresWireMessage::ResultSetRow { cols } => {
                cur_query = None;
                cur_rs_row_count += 1;
                if cur_rs_row_count < 5 {
                    cur_rs_first_rows.push(cols);
                }
            }
            PostgresWireMessage::ReadyForQuery => {
                cur_query = None;
                // new query...
                if cur_rs_row_count > 0 {
                    r.push(MessageData::Postgres(PostgresMessageData::ResultSet {
                        row_count: cur_rs_row_count,
                        first_rows: cur_rs_first_rows,
                    }));
                    cur_rs_row_count = 0;
                    cur_rs_first_rows = vec![];
                }
            }
        }
    }
    r
}

#[derive(Msg)]
pub enum Msg {}

pub struct Model {
    data: PostgresMessageData,
}

#[widget]
impl Widget for PostgresCommEntry {
    fn model(relm: &relm::Relm<Self>, data: PostgresMessageData) -> Model {
        Model { data }
    }

    fn update(&mut self, event: Msg) {}

    fn display_str(data: &PostgresMessageData) -> String {
        match &data {
            PostgresMessageData::Query {
                query,
                parameter_values,
            } if !parameter_values.is_empty() => format!(
                "{}\nparameters: {}",
                query
                    .as_deref()
                    .unwrap_or("Failed retrieving the query string"),
                parameter_values.join(", ")
            ),
            PostgresMessageData::Query {
                query,
                parameter_values: _,
            } => query
                .as_ref()
                .cloned()
                .unwrap_or_else(|| "Failed retrieving the query string".to_string()),
            PostgresMessageData::ResultSet {
                row_count,
                first_rows,
            } => {
                format!(
                    "{} row(s)\n{}",
                    row_count,
                    first_rows
                        .iter()
                        .map(|r| r.join(", "))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }

    view! {
        gtk::Box {
            orientation: gtk::Orientation::Vertical,
            gtk::Separator {},
            gtk::Label {
                label: &PostgresCommEntry::display_str(&self.model.data),
                xalign: 0.0
            },
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
                 "pgsql.val.data": "50:6f:73:74:67:72:65:53:51:4c"
             }
          },
          {
             "pgsql.type": "Data row",
             "pgsql.field.count": "1",
             "pgsql.field.count_tree": {
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
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData::Query {
            query: Some("select 1".to_string()),
            parameter_values: vec![],
        }),
        MessageData::Postgres(PostgresMessageData::ResultSet {
            row_count: 2,
            first_rows: vec![
                vec!["PostgreSQL".to_string()],
                vec!["9.6.12 on x8".to_string()],
            ],
        }),
    ];
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
        MessageData::Postgres(PostgresMessageData::Query {
            query: Some("select 1".to_string()),
            parameter_values: vec![],
        }),
        MessageData::Postgres(PostgresMessageData::Query {
            query: Some("select 1".to_string()),
            parameter_values: vec![],
        }),
        MessageData::Postgres(PostgresMessageData::ResultSet {
            row_count: 1,
            first_rows: vec![vec!["PostgreSQL".to_string()]],
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
        MessageData::Postgres(PostgresMessageData::Query {
            query: Some("select $1".to_string()),
            parameter_values: vec!["TRUE".to_string()],
        }),
        MessageData::Postgres(PostgresMessageData::ResultSet {
            row_count: 1,
            first_rows: vec![vec!["PostgreSQL".to_string()]],
        }),
    ];
    assert_eq!(expected, parsed);
}
