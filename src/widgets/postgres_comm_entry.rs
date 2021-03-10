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

// TODO split the parsing enums and the rendering enums?
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostgresMessageData {
    Query {
        query: Option<String>,
        statement: Option<String>,
        // parameter_values: Vec<String>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery,
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
fn parse_pg_value(pgsql: &serde_json::Value) -> Option<PostgresMessageData> {
    let obj = pgsql.as_object();
    let typ = obj
        .and_then(|o| o.get("pgsql.type"))
        .and_then(|t| t.as_str());
    // if let Some(query_info) = obj.and_then(|o| o.get("pgsql.query")) {
    if typ == Some("Parse") {
        return Some(PostgresMessageData::Query {
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
        return Some(PostgresMessageData::Query {
            // query: format!("{} -> {}", time_relative, q),
            query: None,
            statement: obj
                .and_then(|o| o.get("pgsql.statement"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
        });
    }
    if typ == Some("Ready for query") {
        return Some(PostgresMessageData::ReadyForQuery);
    }
    if typ == Some("Data row") {
        // println!("{:?}", rs);
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
        return Some(PostgresMessageData::ResultSetRow { cols });
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

fn merge_message_datas(mds: Vec<PostgresMessageData>) -> Vec<MessageData> {
    let mut r = vec![];
    let mut cur_query_st = None;
    let mut cur_rs_row_count = 0;
    let mut cur_rs_first_rows = vec![];
    let mut known_statements = HashMap::new();
    for md in mds {
        match md {
            PostgresMessageData::Query {
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
                if statement.is_none() || (cur_query_st.as_ref() != statement.as_ref()) {
                    if let (Some(st), Some(q)) = (statement, query) {
                        known_statements.insert((*st).clone(), (*q).clone());
                    }
                    let query_with_fallback = match (query, statement) {
                        (Some(_), _) => query.clone(),
                        (None, Some(s)) => known_statements.get(s).cloned(),
                        _ => None,
                    };
                    if query_with_fallback.is_some() {
                        r.push(MessageData::Postgres(PostgresMessageData::Query {
                            statement: statement.clone(),
                            query: query_with_fallback,
                        }));
                    }
                    cur_query_st = statement.clone();
                }
            }
            PostgresMessageData::ResultSetRow { cols } => {
                cur_query_st = None;
                cur_rs_row_count += 1;
                if cur_rs_row_count < 5 {
                    cur_rs_first_rows.push(cols);
                }
            }
            PostgresMessageData::ReadyForQuery => {
                cur_query_st = None;
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
            PostgresMessageData::ResultSet {
                row_count,
                first_rows,
            } => panic!(),
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
            PostgresMessageData::Query { query, statement } => {
                query.as_ref().cloned().unwrap_or_else(|| "".to_string())
            }
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
            PostgresMessageData::ResultSetRow { cols } => panic!(),
            PostgresMessageData::ReadyForQuery => panic!(),
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
    match json {
        serde_json::Value::Array(vals) => vals.iter().collect(),
        _ => panic!(),
    }
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
            statement: None,
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
            statement: Some("S_18".to_string()),
        }),
        MessageData::Postgres(PostgresMessageData::Query {
            query: Some("select 1".to_string()),
            statement: Some("S_18".to_string()),
        }),
        MessageData::Postgres(PostgresMessageData::ResultSet {
            row_count: 1,
            first_rows: vec![vec!["PostgreSQL".to_string()]],
        }),
    ];
    assert_eq!(expected, parsed);
}
