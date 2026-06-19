use std::{cell::BorrowError, collections::HashMap, error::Error, fmt::Display, ops::AddAssign, sync::{Arc, atomic::AtomicU8}};
use tokio::{io::{AsyncRead, BufReader}, sync::{Mutex, mpsc::{Receiver, Sender, channel, error::SendError}}};
use quick_xml::{Error as XmlError, Reader, events::{BytesStart, Event}};

use crate::{rulebuilder::{Node, NodeView, Path, Rule, Tree}, xmlworker::FileResult};

#[derive(Debug)]
pub struct TechnicalError {
    pub error: String,
    pub file: String
}

#[derive(Debug)]
pub struct XmlWorkload {
    pub file_id: u64,
    pub workload_counter: u8,
    pub file: String,
    pub tag: String,
    pub path: Vec<Path>,
    pub rules: Vec<Box<dyn Rule>>,
    pub events: Receiver<NodeView>
}

#[derive(Debug)]
pub enum ReaderState {
    Reading,
    Ignoring
}

#[derive(Debug)]
pub struct PathIndex(u32);

impl AddAssign<u32> for PathIndex {
    fn add_assign(&mut self, rhs: u32) {
        self.0 += rhs;
    }
}

impl From<&PathIndex> for usize {
    fn from(value: &PathIndex) -> Self {
        value.0 as usize
    }
}

#[derive(Debug)]
pub struct ReaderContext<'a> {
    current_path: Vec<Path>,
    mode: ReaderState,
    ignore_path: Vec<Path>,
    element_senders: HashMap<Vec<Path>, (Sender<NodeView>, PathIndex)>,
    current_descriptor: &'a Node,
    current_view: Vec<NodeView>
}

impl <'a> ReaderContext<'a> {
    pub fn new(descriptor: &'a Node) -> Self {
        Self {
            current_descriptor: descriptor,
            current_path: Vec::new(),
            mode: ReaderState::Reading,
            ignore_path: Vec::new(),
            current_view: Vec::new(),
            element_senders: HashMap::new()
        }
    }

    pub fn match_child_swap_descriptor(&mut self, tree: &'a Tree, path: &Path) -> bool {

        let mut maybe_child: Option<&Node> = None;
        let matched = match tree.children(self.current_descriptor).get(path) {
            Some(child) => {
                maybe_child = Some(child);
                true
            },
            None => false
        };
        match maybe_child {
            Some(child) => {
                self.current_descriptor = child;
            },
            None => {}
        };
        matched
    }

    pub fn set_parent_swap_descriptor(&mut self, tree: &'a Tree) {
        match tree.parent(self.current_descriptor) {
            Some(parent) => {
                self.current_descriptor = parent;
            },
            None => {}
        }
    }
}

#[derive(Debug)]
pub struct FileReaderError(String);

impl Display for FileReaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}
impl Error for FileReaderError {}

impl From<XmlError> for FileReaderError {
    fn from(value: XmlError) -> Self {
        FileReaderError(value.to_string())
    }
}

impl <T> From<SendError<T>> for FileReaderError {
    fn from(value: SendError<T>) -> Self {
        FileReaderError(value.to_string())
    }
}

impl From<BorrowError> for FileReaderError {
    fn from(value: BorrowError) -> Self {
        FileReaderError(value.to_string())
    }
}

pub async fn read<S>(
    file_id: u64,
    file: &str,
    src: S,
    descriptors: Arc<Tree>, 
    sender: &Sender<XmlWorkload>,
    collector_sender: &Sender<FileResult>,
    error_sender: &Sender<TechnicalError>,
    highwatermark: usize
) -> Result<(), FileReaderError>
where
    S: AsyncRead + Unpin,
{
    let workload_counter_seq = Arc::new(AtomicU8::new(0));
    let reader_context: Mutex<ReaderContext> = Mutex::new(ReaderContext::new(descriptors.get_root().ok_or(FileReaderError("empty descriptors".into()))?));
    let reader = BufReader::new(src);
    let mut reader = Reader::from_reader(reader);
    let mut read_buf = Vec::new();
    reader.config_mut().trim_text(true);

    loop {
        let mut reader_context =  reader_context.lock().await;
        match reader.read_event_into_async(&mut read_buf).await? {
            Event::Start(tag) => {
                let tag_path = Path(String::from_utf8_lossy(tag.name().as_ref()).into());
                match &reader_context.mode {
                    ReaderState::Ignoring => {
                        // if in ignoring state, we are skipping children
                        // since switching out of ignore mode is made at the end tag, we can
                        // safely add this tag to the ignore vector
                        reader_context.ignore_path.push(tag_path);
                    },
                    ReaderState::Reading => {
                        // if we hit the current descriptor path, we need to create a stream,
                        // register it and send the payload to the pool
                        // of workers
                        if reader_context.current_descriptor.path().0.as_bytes() == tag.name().as_ref() {
                            handle_path_match(
                                file_id,
                                file,
                                Arc::clone(&workload_counter_seq),
                                tag,
                                &mut reader_context,
                                sender,
                                error_sender,
                                highwatermark
                            ).await?;
                        } else {
                            // if not, we must look into the descriptor children to check
                            // for any match
                            
                            match reader_context.match_child_swap_descriptor(&descriptors, &tag_path) {
                                true => {
                                    // if found, we must trigger the same chain of events as for the matching tag one
                                    handle_path_match(
                                        file_id,
                                        file,
                                        Arc::clone(&workload_counter_seq),
                                        tag,
                                        &mut reader_context,
                                        sender,
                                        error_sender,
                                        highwatermark
                                    ).await?;
                                },
                                false => {
                                    // if no child is a match, we must ignore this tag entirely,
                                    // including his children, until we meet the closing tag
                                    // first, we set up the mode, so that we skip early in the loop, and track depth
                                    // second, we insert the first entry of the dive
                                    reader_context.mode = ReaderState::Ignoring;
                                    reader_context.ignore_path.push(tag_path);
                                }
                            }
                        }
                    }
                };
            },
            Event::End(tag) => {
                match reader_context.mode {
                    ReaderState::Ignoring => {
                        // if the current ignore vector length is one,
                        // and the tag is the stored path,
                        // we then switch to reading mode
                        if
                            reader_context.ignore_path.len() == 1
                            && reader_context.ignore_path[0] == Path(String::from_utf8_lossy(tag.name().as_ref()).into())
                        {
                            reader_context.mode = ReaderState::Reading;
                        }
                        reader_context.ignore_path.pop();
                    },
                    ReaderState::Reading => {
                        // if we are reading, we need to send the upppermost view in construction
                        match reader_context.current_view.pop() {
                            Some(view) => {
                                match reader_context.element_senders.get(&reader_context.current_path) {
                                    Some((sender, _)) => {
                                        sender.send(view).await?;
                                    },
                                    None => {}
                                };
                                // we must pop the current path also, to ensure proper tracking
                                reader_context.current_path.pop();
                                // finally, we must restore the parent descriptor as current descriptor
                                reader_context.set_parent_swap_descriptor(&descriptors)
                            },
                            None => {}
                        }

                    }
                }
            },
            Event::Text(text) => {
                // if we match any text, we must add it to the top of the pill view
                let length = reader_context.current_view.len();
                let view = &mut reader_context.current_view[length - 1];
                view.set_text(String::from_utf8_lossy(&text.into_inner()).trim());
            },
            Event::Eof => {
                // we need to send the termination of file to the collector so that it can emit the diagnostic
                match collector_sender.send(FileResult::Terminated(file_id, file.into(), workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1)).await {
                    Ok(_) => {},
                    Err(err) => {
                        match &err.0 {
                            FileResult::Progress(_, file_name, _, _) |
                            FileResult::Terminated(_, file_name, _) => {
                                error_sender.send(TechnicalError {
                                    error: err.to_string(),
                                    file: file_name.to_string()
                                }).await?
                            }
                        }
                    }
                };
                break;
            },
            _ => { continue; }
        }
    };
    Ok(())
}

async fn handle_path_match<'a, 'b>(
    file_id: u64,
    file: &str,
    workload_id_seq: Arc<AtomicU8>,
    tag: BytesStart<'_>,
    reader_context: &'b mut ReaderContext<'a>,
    sender: &Sender<XmlWorkload>,
    error_sender: &Sender<TechnicalError>,
    highwatermark: usize
) -> Result<(), FileReaderError> {
    // if there is no current channel, initiate one.
    // also push a new index to the stack
    // as long as the path
    let path = reader_context.current_descriptor.path().clone();
    reader_context.current_path.push(path);
    if !reader_context.element_senders.contains_key(&reader_context.current_path) {
        // creating the channel
        let (tx, rx) = channel::<NodeView>(highwatermark);
        //send the worload, including the event receiver
        match sender.send(
            XmlWorkload {
                file_id,
                workload_counter: workload_id_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                file: file.to_string(),
                path: reader_context.current_path.clone(),
                tag: String::from_utf8_lossy(tag.name().into_inner()).to_string(),
                rules: reader_context.current_descriptor.rules(),
                events: rx,
            }
        ).await {
            Ok(_e) => {},
            Err(err) => {
                // if we fail to send the workload, we must
                // prematurely stop the validation
                // (technical error)
                error_sender.send(TechnicalError {
                    error: err.to_string(),
                    file: err.0.file
                }).await?
            }
        };
        // we then store the sender on the hashmap, as long as the current element index
        let path = reader_context.current_path.clone();
        reader_context.element_senders.insert(path, (tx, PathIndex(0)));
    } else {
        // reader_context.current_path.pop();
        // if there is already one, it's a new element of a stream, we need to
        // increment the current index
        match reader_context.element_senders.get_mut(&reader_context.current_path) {
            Some((_, index)) => {
                *index += 1
            },
            None => {}
        };
    }
    // in case the element contains a text content,
    // we need to keep it until the end tag is met
    let mut attrs = HashMap::new();
    for attr in tag.attributes() {
        match attr {
            Ok(attr) => {
                attrs.insert(
                    String::from_utf8_lossy(attr.key.as_ref()).into(),
                    String::from_utf8_lossy(attr.value.as_ref()).into()
                );
            },
            // we ignore malformed attributes
            Err(_e) => {}
        };
    }
    match reader_context.element_senders.get(&reader_context.current_path) {
            Some((_, index)) => {
                reader_context.current_view.push(NodeView::new(attrs, index.into()));
            },
            None => {
                reader_context.current_view.push(NodeView::new(attrs, 0));
            }
        }
    Ok(())
}