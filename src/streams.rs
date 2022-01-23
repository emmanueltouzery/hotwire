use crate::{
    http::http_message_parser::{Http, HttpMessageData, HttpStreamGlobals},
    http2::http2_message_parser::{Http2, Http2StreamGlobals},
    message_parser::{FromToAnyMessages, MessageParser, StreamData},
    pgsql::postgres_message_parser::{Postgres, PostgresMessageData, PostgresStreamGlobals},
    widgets::comm_target_card::CommTargetCardKey,
};
use std::{collections::HashMap, net::IpAddr};

use crate::{
    message_parser::ClientServerInfo,
    tshark_communication::{TSharkPacket, TcpStreamId},
};

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum SessionChangeType {
    NewSession,
    NewDataInSession,
}

pub trait Streams {
    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String>;
    fn handle_got_packet(
        &mut self,
        p: TSharkPacket,
    ) -> Result<
        (
            usize,
            SessionChangeType,
            Option<ClientServerInfo>,
            usize,
            Option<&str>,
            bool,
        ),
        (TcpStreamId, String),
    >;
    fn messages_len(&self, stream_id: TcpStreamId) -> usize;
    fn client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo>;
    fn protocol_index(&self, stream_id: TcpStreamId) -> Option<usize>;

    fn by_remote_ip(
        &self,
        card_key: CommTargetCardKey,
        constrain_remote_ips: &[IpAddr],
        constrain_stream_ids: &[TcpStreamId],
    ) -> HashMap<IpAddr, Vec<TcpStreamId>>;
}

pub type StreamsImpl = (
    HashMap<TcpStreamId, StreamData<HttpStreamGlobals, Vec<HttpMessageData>>>,
    HashMap<TcpStreamId, StreamData<PostgresStreamGlobals, Vec<PostgresMessageData>>>,
    HashMap<TcpStreamId, StreamData<Http2StreamGlobals, Vec<HttpMessageData>>>,
);

// TODO this is meant to be implemented through a macro
impl Streams for StreamsImpl {
    fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String> {
        if let Some(s) = self.0.get_mut(&stream_id) {
            return Http.finish_stream(*s).map(|_| ());
        }
        if let Some(s) = self.1.get(&stream_id) {
            return Postgres.finish_stream(*s).map(|_| ());
        }
        if let Some(s) = self.2.get(&stream_id) {
            return Http2.finish_stream(*s).map(|_| ());
        }
        panic!();
    }

    // TODO obviously return a struct not a huge tuple like that
    fn handle_got_packet(
        &mut self,
        p: TSharkPacket,
    ) -> Result<
        (
            usize,
            SessionChangeType,
            Option<ClientServerInfo>,
            usize,
            Option<&str>,
            bool,
        ),
        (TcpStreamId, String),
    > {
        if Http.is_my_message(&p) {
            return parser_handle_got_packet(Http, 0, p, &mut self.0);
        }
        if Postgres.is_my_message(&p) {
            return parser_handle_got_packet(Postgres, 1, p, &mut self.1);
        }
        if Http2.is_my_message(&p) {
            return parser_handle_got_packet(Http2, 2, p, &mut self.2);
        }
        panic!();
    }

    fn messages_len(&self, stream_id: TcpStreamId) -> usize {
        if let Some(sd) = self.0.get(&stream_id) {
            return sd.messages.len();
        }
        if let Some(sd) = self.1.get(&stream_id) {
            return sd.messages.len();
        }
        if let Some(sd) = self.2.get(&stream_id) {
            return sd.messages.len();
        }
        panic!();
    }

    fn client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo> {
        if let Some(sd) = self.0.get(&stream_id) {
            return sd.client_server;
        }
        if let Some(sd) = self.1.get(&stream_id) {
            return sd.client_server;
        }
        if let Some(sd) = self.2.get(&stream_id) {
            return sd.client_server;
        }
        panic!();
    }

    fn protocol_index(&self, stream_id: TcpStreamId) -> Option<usize> {
        if let Some(sd) = self.0.get(&stream_id) {
            return Some(sd.parser_index);
        }
        if let Some(sd) = self.1.get(&stream_id) {
            return Some(sd.parser_index);
        }
        if let Some(sd) = self.2.get(&stream_id) {
            return Some(sd.parser_index);
        }
        None
    }

    fn by_remote_ip(
        &self,
        card_key: CommTargetCardKey,
        constrain_remote_ips: &[IpAddr],
        constrain_stream_ids: &[TcpStreamId],
    ) -> HashMap<IpAddr, Vec<TcpStreamId>> {
    }
    // let mut by_remote_ip = HashMap::new();
    // let parsers = win::get_message_parsers();
    // for (stream_id, messages) in streams {
    //     if !matches!(messages.client_server, Some(cs) if card.to_key().matches_server(cs)) {
    //         continue;
    //     }
    //     let allowed_all = constrain_remote_ips.is_empty() && constrain_stream_ids.is_empty();

    //     let allowed_ip = messages
    //         .client_server
    //         .as_ref()
    //         .filter(|cs| constrain_remote_ips.contains(&cs.client_ip))
    //         .is_some();
    //     let allowed_stream = constrain_stream_ids.contains(stream_id);
    //     let allowed = allowed_all || allowed_ip || allowed_stream;

    //     if !allowed {
    //         continue;
    //     }
    //     let remote_server_streams = by_remote_ip
    //         .entry(
    //             messages
    //                 .client_server
    //                 .as_ref()
    //                 .map(|cs| cs.client_ip)
    //                 .unwrap_or_else(|| IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))),
    //         )
    //         .or_insert_with(Vec::new);
    //     remote_server_streams.push((stream_id, messages));
    // }
}

fn parser_handle_got_packet<MP: MessageParser>(
    parser: MP,
    parser_idx: usize,
    p: TSharkPacket,
    streams: &mut HashMap<TcpStreamId, StreamData<MP::StreamGlobalsType, MP::MessagesType>>,
) -> Result<
    (
        usize,
        SessionChangeType,
        Option<ClientServerInfo>,
        usize,
        Option<&str>,
        bool,
    ),
    (TcpStreamId, String),
> {
    let packet_stream_id = p.basic_info.tcp_stream_id;
    let existing_stream = streams.remove(&packet_stream_id);
    let message_count_before;
    let session_change_type;
    let is_new_stream;
    let stream_data = if let Some(stream_data) = existing_stream {
        // existing stream
        is_new_stream = false;
        session_change_type = SessionChangeType::NewDataInSession;
        message_count_before = stream_data.messages.len();
        let stream_data = match parser.add_to_stream(stream_data, p) {
            Ok(sd) => sd,
            Err(msg) => {
                return Err((packet_stream_id, msg));
            }
        };
        stream_data
    } else {
        // new stream
        is_new_stream = true;
        session_change_type = SessionChangeType::NewSession;
        message_count_before = 0;
        let mut stream_data = StreamData {
            parser_index: parser_idx,
            stream_globals: parser.initial_globals(),
            client_server: None,
            messages: parser.empty_messages_data(),
            summary_details: None,
        };
        match parser.add_to_stream(stream_data, p) {
            Ok(sd) => {
                stream_data = sd;
            }
            Err(msg) => {
                return Err((packet_stream_id, msg));
            }
        }
        stream_data
    };
    Ok((
        parser_idx,
        session_change_type,
        stream_data.client_server,
        message_count_before,
        stream_data.summary_details.as_deref(),
        is_new_stream,
    ))
}
