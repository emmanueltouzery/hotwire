// https://www.postgresql.org/docs/12/protocol.html
use crate::tshark_communication;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::io::BufReader;
use std::process::ChildStdout;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PostgresColType {
    Bool,
    Name,
    Char,
    Text,
    Oid,
    Varchar,
    Int2,
    Int4,
    Int8,
    Timestamp,
    Other,
}

#[derive(Debug)]
pub enum PostgresWireMessage {
    Startup {
        username: Option<String>,
        database: Option<String>,
        application: Option<String>,
    },
    CopyData,
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
        col_types: Vec<PostgresColType>,
    },
    ResultSetRow {
        cols: Vec<String>,
    },
    ReadyForQuery,
}

fn parse_pgsql_info(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> Vec<PostgresWireMessage> {
    let result = vec![];
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"pgsql.type") => {
                            match tshark_communication::element_attr_val(e, b"show") {
                                b"Startup message" => {
                                    result.push(parse_startup_message(xml_reader, buf))
                                }
                                b"Copy data" => result.push(PostgresWireMessage::CopyData),
                                b"Parse" => result.push(parse_parse_message(xml_reader, buf)),
                            }
                        }
                    }
                }
            }
        }
    }
}

fn parse_startup_message(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> PostgresWireMessage {
    let mut cur_param_name;
    let mut username;
    let mut database;
    let mut application;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    let val = tshark_communication::element_attr_val(e, b"show");
                    match name {
                        Some(b"pgsql.parameter_name") => {
                            cur_param_name = val;
                        }
                        Some(b"pgsql.parameter_value") => match cur_param_name {
                            b"user" => {
                                username = String::from_utf8(val.to_vec()).ok();
                            }
                            b"database" => {
                                database = String::from_utf8(val.to_vec()).ok();
                            }
                            b"application_name" => {
                                application = String::from_utf8(val.to_vec()).ok();
                            }
                        },
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return PostgresWireMessage::Startup {
                        username,
                        database,
                        application,
                    };
                }
            }
        }
    }
}

fn parse_parse_message(
    xml_reader: &quick_xml::Reader<BufReader<ChildStdout>>,
    buf: &mut Vec<u8>,
) -> PostgresWireMessage {
    let mut statement;
    let mut query;
    loop {
        match xml_reader.read_event(buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name() == b"field" {
                    let name = e
                        .attributes()
                        .find(|kv| kv.unwrap().key == "name".as_bytes())
                        .map(|kv| &*kv.unwrap().value);
                    match name {
                        Some(b"pgsql.statement") => {
                            statement = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .ok();
                        }
                        Some(b"pgsql.query") => {
                            query = String::from_utf8(
                                tshark_communication::element_attr_val(e, b"show").to_vec(),
                            )
                            .ok();
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.name() == b"proto" {
                    return PostgresWireMessage::Parse { statement, query };
                }
            }
        }
    }
}

fn parse_pg_value(pgsql: &serde_json::Value) -> Option<PostgresWireMessage> {
    let obj = pgsql.as_object();
    let typ = obj
        .and_then(|o| o.get("pgsql.type"))
        .and_then(|t| t.as_str());
    // if let Some(query_info) = obj.and_then(|o| o.get("pgsql.query")) {
    match typ {
        Some("Bind") => {
            // for prepared statements, the first time we get parse & statement+query
            // the following times we get bind and only statement (statement ID)
            // we can then recover the query from the statement id in post-processing.
            Some(PostgresWireMessage::Bind {
                // query: format!("{} -> {}", time_relative, q),
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
            })
        }
        Some("Ready for query") => Some(PostgresWireMessage::ReadyForQuery),
        Some("Row description") => {
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
            let col_types = match tree.get("pgsql.col.name_tree") {
                Some(serde_json::Value::Object(o)) => {
                    vec![o
                        .get("pgsql.oid.type")
                        .and_then(|t| t.as_str())
                        .map(parse_pg_oid_type)
                        .unwrap_or(PostgresColType::Other)]
                }
                Some(serde_json::Value::Array(ar)) => ar
                    .iter()
                    .map(|v| {
                        v.as_object()
                            .and_then(|o| o.get("pgsql.oid.type"))
                            .and_then(|t| t.as_str())
                            .map(parse_pg_oid_type)
                            .unwrap_or(PostgresColType::Other)
                    })
                    .collect(),
                _ => vec![],
            };
            Some(PostgresWireMessage::RowDescription {
                col_names,
                col_types,
            })
        }
        Some("Data row") => {
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
            Some(PostgresWireMessage::ResultSetRow { cols })
        }
        _ => None,
    }
}

/// select * from postgres.pg_catalog.pg_type
fn parse_pg_oid_type(typ: &str) -> PostgresColType {
    match typ.parse() {
        Ok(16) => PostgresColType::Bool,
        Ok(18) => PostgresColType::Char,
        Ok(19) => PostgresColType::Name,
        Ok(20) => PostgresColType::Int8,
        Ok(21) => PostgresColType::Int2,
        Ok(23) => PostgresColType::Int4,
        Ok(25) => PostgresColType::Text,
        Ok(26) => PostgresColType::Oid,
        Ok(1043) => PostgresColType::Varchar,
        Ok(1114) => PostgresColType::Timestamp,
        _ => {
            eprintln!("Unhandled postgres type: {:?}", typ);
            PostgresColType::Other
        }
    }
}

fn as_string_array(val: &serde_json::Value) -> Option<Vec<&str>> {
    val.as_array()
        .and_then(|v| v.into_iter().map(|i| i.as_str()).collect())
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
