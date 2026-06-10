use std::collections::HashMap;
use futures::Stream;
use tokio_util::{bytes::Buf, io::StreamReader};
use tokio::sync::mpsc::{Sender, channel, Receiver};
use quick_xml::{Reader, events::Event, Error};

use crate::rulebuilder::{Node, NodeView, Path, Rule};

pub struct XmlWorkload {
    file: String,
    tag: String,
    rules: Vec<Box<dyn Rule>>,
    events: Receiver<NodeView>,
}

pub struct FileReader;

impl FileReader {
    pub async fn stream_validation<S, B, E>(
        file: &str,
        src: S,
        descriptors: &Node<'_>, 
        sender: Sender<XmlWorkload>,
        highwatermark: usize
    ) -> Result<(), Error>
    where
        S: Stream<Item = Result<B, E>> + Unpin,
        B: Buf,
        E: Into<std::io::Error>,
    {
        let mut current_path = Vec::from([descriptors.path().clone()]);
        let mut current_path_index: u32 = 0;
        let mut elements_streams: HashMap<Vec<Path>, Sender<NodeView>>;
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
                    if current_descriptor.path().0.as_bytes() == tag.name().as_ref() {
                        // if there is no current streams, initiate one.
                        if !elements_streams.contains_key(&current_path) {
                            let (tx, mut rx) = channel::<NodeView>(highwatermark);
                            sender.send(
                                XmlWorkload {
                                    file: file.to_string(),
                                    tag: String::from_utf8_lossy(tag.name().into_inner()).to_string(),
                                    rules: current_descriptor.rules(),
                                    events: rx,
                                }
                            ).await;
                            tx.send(
                                NodeView {
                                    
                                }
                            );
                        }
                    
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
                    break;
                },
                _ => { continue; }
            }
        };
        Ok(())
    }
}