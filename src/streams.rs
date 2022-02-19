use crate::{
    custom_streams_store::{ClientServerInfo, CustomStreamsStore},
    http::http_streams_store::HttpStreamsStore,
    http2::http2_streams_store::Http2StreamsStore,
    pgsql::postgres_streams_store::PostgresStreamsStore,
    tshark_communication::{TSharkPacket, TcpStreamId},
};
use itertools::Itertools;

pub struct Streams {
    // this field name is 200% wrong
    streams: Vec<Box<dyn CustomStreamsStore>>,
}

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum SessionChangeType {
    NewSession,
    NewDataInSession,
}

impl Default for Streams {
    fn default() -> Streams {
        Streams {
            streams: vec![
                Box::new(HttpStreamsStore::default()),
                Box::new(PostgresStreamsStore::default()),
                Box::new(Http2StreamsStore::default()),
            ],
        }
    }
}

pub struct PacketAddedData {
    pub store_index: usize,
    pub message_count_before: usize,
    pub session_change_type: SessionChangeType,
    pub client_server_info: Option<ClientServerInfo>,
}

impl Streams {
    pub fn get_streams_stores(&self) -> &[Box<dyn CustomStreamsStore>] {
        &self.streams
    }

    pub fn get_streams_stores_mut(&mut self) -> &mut [Box<dyn CustomStreamsStore>] {
        &mut self.streams
    }

    pub fn get_streams_store(&self, store_index: usize) -> &Box<dyn CustomStreamsStore> {
        self.streams.get(store_index).unwrap()
    }

    pub fn tcp_stream_ids(&self) -> Vec<TcpStreamId> {
        self.streams
            .iter()
            .flat_map(|s| s.tcp_stream_ids())
            .collect()
    }

    pub fn stream_message_count(&self, stream_id: TcpStreamId) -> Option<usize> {
        self.streams
            .iter()
            .find_map(|s| s.stream_message_count(stream_id))
    }

    pub fn stream_summary_details(&self, stream_id: TcpStreamId) -> Option<&str> {
        self.streams
            .iter()
            .find_map(|s| s.stream_summary_details(stream_id))
    }

    pub fn get_store_index(&self, stream_id: TcpStreamId) -> Option<usize> {
        self.streams.iter().position(|s| s.has_stream_id(stream_id))
    }

    pub fn get_client_server(&self, stream_id: TcpStreamId) -> Option<ClientServerInfo> {
        self.streams
            .iter()
            .find_map(|s| s.stream_client_server(stream_id))
    }

    pub fn supported_string_filter_keys(&self, store_index: usize) -> &'static [&'static str] {
        self.streams
            .get(store_index)
            .unwrap()
            .supported_string_filter_keys()
    }

    pub fn supported_numeric_filter_keys(&self, store_index: usize) -> &'static [&'static str] {
        self.streams
            .get(store_index)
            .unwrap()
            .supported_numeric_filter_keys()
    }

    pub fn finish_stream(&mut self, stream_id: TcpStreamId) -> Result<(), String> {
        self.streams
            .iter_mut()
            .find(|s| s.has_stream_id(stream_id))
            .map(|s| s.finish_stream(stream_id))
            .unwrap_or_else(|| Err("no such stream".to_string()))
    }

    pub fn clear(&mut self) {
        for mp in &mut self.streams {
            mp.reset();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.streams.iter().all(|mp| mp.is_empty())
    }

    pub fn tshark_filter_string(&self) -> String {
        self.streams
            .iter()
            .map(|p| p.tshark_filter_string())
            .join(" || ")
    }

    fn get_stream_store_for_packet(
        &mut self,
        p: &TSharkPacket,
    ) -> Option<(usize, &mut Box<dyn CustomStreamsStore>)> {
        if let Some(store_index) = self.get_store_index(p.basic_info.tcp_stream_id) {
            Some((store_index, self.streams.get_mut(store_index).unwrap()))
        } else {
            self.streams
                .iter_mut()
                .enumerate()
                .find(|(_idx, ps)| ps.is_my_message(p))
        }
    }

    pub fn handle_got_packet(
        &mut self,
        p: TSharkPacket,
    ) -> Result<Option<PacketAddedData>, String> {
        if let Some((store_index, store)) = self.get_stream_store_for_packet(&p) {
            let packet_stream_id = p.basic_info.tcp_stream_id;
            let message_count_before = store.stream_message_count(packet_stream_id).unwrap_or(0);
            let session_change_type = if message_count_before > 0 {
                // existing stream
                SessionChangeType::NewDataInSession
            } else {
                // new stream
                SessionChangeType::NewSession
            };
            store
                .add_to_stream(packet_stream_id, p)
                .map(|client_server_info| {
                    Some(PacketAddedData {
                        store_index,
                        message_count_before,
                        session_change_type,
                        client_server_info,
                    })
                })
        } else {
            Ok(None)
        }
    }
}
