use std::{collections::HashMap, sync::Arc};
use futures::Stream;
use tokio_util::{bytes::Buf, io::StreamReader};
use quick_xml::{Reader, events::Event, Error};

use crate::rulebuilder::{Node, NodeView};

pub struct FileReader;

impl FileReader {
    pub async fn stream_validation<S, B, E>(src: S, descriptors: &Node<'_>) -> Result<(), Error>
    where
        S: Stream<Item = Result<B, E>> + Unpin,
        B: Buf,
        E: Into<std::io::Error>,
    {
        let mut elements_streams: HashMap<Vec<String>, Arc<dyn Stream<Item = NodeView>>>;
        let mut current_descriptor = descriptors;
        let reader = StreamReader::new(src);
        let mut reader = Reader::from_reader(reader);
        let mut read_buf = Vec::new();
        reader.config_mut().trim_text(true);

        loop {
            match reader.read_event_into_async(&mut read_buf).await? {
                Event::Start(tag) => {
                    // if we hit the current descriptor path, we need to create a stream,
                    // register it and send the payload to the pool
                    // of workers
                    if current_descriptor.path().as_bytes() == tag.name().as_ref() {

                    
                    } else {
                        // if not, we mut look into the descriptor children to check
                        // for any match
                        let candidate = 
                    }
                },
                Event::End(tag) => {

                },
                Event::Text(text) => {

                },
                Event::Eof => {

                },
                _ => { continue; }
            }
        }
    }
}