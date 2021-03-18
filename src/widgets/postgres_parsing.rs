// https://www.postgresql.org/docs/12/protocol.html
use super::postgres_comm_entry::PostgresMessageData;
use crate::widgets::comm_remote_server::MessageData;
use crate::TSharkCommunication;
use chrono::{NaiveDate, NaiveDateTime};
use std::collections::HashMap;

#[cfg(test)]
use crate::tshark_communication::{TSharkFrameLayer, TSharkLayers, TSharkSource};

#[derive(Debug)]
pub enum PostgresWireMessage {
    Parse {
        query: Option<String>,
        statement: Option<String>,
    },
    Bind {
        timestamp: NaiveDateTime,
        statement: Option<String>,
        parameter_values: Vec<String>,
    },
    RowDescription {
        col_names: Vec<String>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery {
        timestamp: NaiveDateTime,
    },
}

pub fn parse_pg_stream(
    all_vals: Vec<(&TSharkCommunication, &serde_json::Value)>,
) -> Vec<MessageData> {
    let decoded_messages = all_vals
        .into_iter()
        .filter_map(|(p, v)| parse_pg_value(p, v))
        .collect();
    merge_message_datas(decoded_messages)
}

// now postgres bound parameters.. $1, $2..
// for instance in session 34
fn parse_pg_value(
    packet: &TSharkCommunication,
    pgsql: &serde_json::Value,
) -> Option<PostgresWireMessage> {
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
            timestamp: packet.source.layers.frame.frame_time,
            statement: obj
                .and_then(|o| o.get("pgsql.statement"))
                .and_then(|s| s.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string()),
            parameter_values: obj
                .and_then(|o| o.iter().find(|(k, _v)| k.starts_with("Parameter values: ")))
                .and_then(|(_k, v)| {
                    if let serde_json::Value::Object(tree) = v {
                        let col_lengths: Vec<i32> =
                            parse_str_or_array(tree.get("pgsql.val.length").unwrap(), |s| {
                                s.parse().unwrap_or(0)
                            });
                        let raw_cols =
                            parse_str_or_array(tree.get("pgsql.val.data").unwrap(), |s| {
                                hex_chars_to_string(s).unwrap_or_default() // TODO should check pgsql.oid.type.. sometimes i get integers here
                            });
                        return Some(add_cols(raw_cols, col_lengths));
                    }
                    None
                })
                .unwrap_or_else(Vec::new),
        });
    }
    if typ == Some("Ready for query") {
        return Some(PostgresWireMessage::ReadyForQuery {
            timestamp: packet.source.layers.frame.frame_time,
        });
    }
    if typ == Some("Row description") {
        let tree = obj.unwrap().get("pgsql.field.count_tree").unwrap();
        let col_names = match tree.get("pgsql.col.name") {
            Some(serde_json::Value::String(s)) => {
                vec![s.to_string()]
            }
            Some(serde_json::Value::Array(ar)) => {
                ar.iter().map(|v| v.as_str().unwrap().to_string()).collect()
            }
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
        let col_lengths: Vec<i32> =
            parse_str_or_array(tree.get("pgsql.val.length").unwrap(), |s| {
                s.parse().unwrap_or(0) // TODO the _or(0) is a workaround because we didn't code everything yet
            });

        let raw_cols = tree
            .get("pgsql.val.data")
            // TODO the or_default is a workaround because we didn't code everything yet
            .map(|t| parse_str_or_array(t, |s| hex_chars_to_string(s).unwrap_or_default()))
            .unwrap_or_else(Vec::new);
        let cols = add_cols(raw_cols, col_lengths);
        if col_count != cols.len() {
            panic!("{} != {}", col_count, cols.len());
        }
        return Some(PostgresWireMessage::ResultSetRow { cols });
    }

    // "pgsql.type": "Bind",
    None
}

fn parse_str_or_array<T>(val: &serde_json::Value, converter: impl Fn(&str) -> T) -> Vec<T> {
    match val {
        serde_json::Value::String(s) => {
            vec![converter(s)]
        }
        serde_json::Value::Array(ar) => ar.iter().map(|v| converter(v.as_str().unwrap())).collect(),
        _ => panic!(),
    }
}

fn add_cols(mut raw_cols: Vec<String>, col_lengths: Vec<i32>) -> Vec<String> {
    raw_cols.reverse();
    let mut cols = vec![];
    for col_length in col_lengths {
        if col_length < 0 {
            cols.push("null".to_string());
        } else if col_length == 0 {
            cols.push("".to_string());
        } else if let Some(val) = raw_cols.pop() {
            cols.push(val);
        }
    }
    if !raw_cols.is_empty() {
        panic!("raw_cols: {:?}", raw_cols);
    }
    cols
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
    let mut query_timestamp = None;
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
                timestamp,
                statement,
                parameter_values,
            } => {
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
            PostgresWireMessage::RowDescription { col_names } => {
                cur_col_names = col_names;
            }
            PostgresWireMessage::ResultSetRow { cols } => {
                cur_rs_row_count += 1;
                cur_rs_first_rows.push(cols);
            }
            PostgresWireMessage::ReadyForQuery { timestamp } => {
                if was_bind {
                    r.push(MessageData::Postgres(PostgresMessageData {
                        query: cur_query_with_fallback,
                        query_timestamp: query_timestamp.unwrap(), // know it was populated since was_bind is true
                        result_timestamp: timestamp,
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
                query_timestamp = None;
            }
        }
    }
    r
}

#[cfg(test)]
fn as_json_array(json: &serde_json::Value) -> Vec<(&TSharkCommunication, &serde_json::Value)> {
    let tshark_test: &'static TSharkCommunication = Box::leak(Box::new(TSharkCommunication {
        source: TSharkSource {
            layers: TSharkLayers {
                frame: TSharkFrameLayer {
                    frame_time: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
                },
                ip: None,
                tcp: None,
                http: None,
                pgsql: None,
            },
        },
    }));
    json.as_array()
        .unwrap()
        .iter()
        .map(|v| (tshark_test, v))
        .collect()
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
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
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
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some("select 1".to_string()),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_first_rows: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
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
        )
        .unwrap(),
    ));
    let expected: Vec<MessageData> = vec![
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some("select $1".to_string()),
            parameter_values: vec![],
            resultset_col_names: vec![],
            resultset_row_count: 0,
            resultset_first_rows: vec![],
        }),
        MessageData::Postgres(PostgresMessageData {
            query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
            query: Some("select $1".to_string()),
            parameter_values: vec!["TRUE".to_string(), "null".to_string(), "TRUER".to_string()],
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
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
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
        query_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
        result_timestamp: NaiveDate::from_ymd(2021, 3, 18).and_hms_nano(0, 0, 0, 0),
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
