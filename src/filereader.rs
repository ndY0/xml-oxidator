use std::{cell::BorrowError, collections::HashMap, error::Error, fmt::Display, ops::AddAssign, sync::{Arc, atomic::AtomicU8}};
use tokio::{io::{AsyncRead, BufReader}, sync::{Mutex, broadcast::{self, Receiver as BroadcastReceiver, Sender as BroadcastSender}, mpsc::{Receiver, Sender, channel, error::SendError}}};
use quick_xml::{Error as XmlError, Reader, events::{BytesStart, Event}};

use crate::{cancellation::ShutdownHandle, init::FatalError, rulebuilder::{CommonNodeView, FullNodeView, Node, PartialNodeView, Path, Rule, Tree}, xmlworker::FileResult};

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
    pub ctx: Arc<Mutex<HashMap<Vec<Path>, PartialNodeView>>>,
    pub events: Receiver<Arc<Mutex<FullNodeView>>>
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
struct SenderContext {
    pub sender: Sender<Arc<Mutex<FullNodeView>>>,
    pub path_index: PathIndex,
}

#[derive(Debug)]
struct CurrentViewContext {
    // pub view: Arc<Mutex<FullNodeView>>,
    pub text_sender: BroadcastSender<String>,
    pub children_sender: HashMap<Vec<Path>, BroadcastSender<PartialNodeView>>
}

#[derive(Debug)]
struct ReaderContext<'a> {
    pub current_path: Vec<Path>,
    pub mode: ReaderState,
    pub ignore_path: Vec<Path>,
    pub element_senders: HashMap<Vec<Path>, SenderContext>,
    pub current_descriptor: &'a Node,
    pub current_view: Vec<CurrentViewContext>,
    pub global_context: Arc<Mutex<HashMap<Vec<Path>, PartialNodeView>>>
}

impl <'a> ReaderContext<'a> {
    pub fn new(descriptor: &'a Node) -> Self {
        Self {
            current_descriptor: descriptor,
            current_path: Vec::new(),
            mode: ReaderState::Reading,
            ignore_path: Vec::new(),
            current_view: Vec::new(),
            element_senders: HashMap::new(),
            global_context: Arc::new(Mutex::new(HashMap::new()))
        }
    }

    pub fn match_child_swap_descriptor(&mut self, tree: &'a Arc<Tree>, path: &Path) -> bool {

        let mut maybe_child: Option<&Node> = None;
        let matched = match tree.children(self.current_descriptor).get(path) {
            Some(child) => {
                maybe_child = Some(child);
                true
            },
            None => false
        };
        if let Some(child) = maybe_child {
            self.current_descriptor = child;
        };
        matched
    }

    pub fn set_parent_swap_descriptor(&mut self, tree: &'a Tree) {
        if let Some(parent) = tree.parent(self.current_descriptor) {
            self.current_descriptor = parent;
        }
    }

    pub fn missed_test_paths(&self, tree: &'a Tree) -> Option<Vec<Vec<Path>>> {
        let root_node = tree.get_root();
        let mut current_path: Vec<Path> = Vec::new();
        let mut missed_paths: Vec<Vec<Path>> = Vec::new();
        root_node.and_then(|node| {
            current_path.push(node.path().clone());
            // if we miss the root node, then we can return early 
            if !self.element_senders.contains_key(&current_path) {
                missed_paths.push(current_path.clone());
                return Some(missed_paths);
            }
            self.collect_missing_path_children(
                &mut missed_paths,
                &mut current_path,
                tree,
                &tree.children(node)
            );
            Some(missed_paths)
        })
    }

    fn collect_missing_path_children(
        &self,
        collector: &mut Vec<Vec<Path>>,
        current_path: &mut Vec<Path>,
        tree: &Tree,
        children: &HashMap<Path, &'a Node>
    ) {
        for (path, &node) in children {
            current_path.push(path.clone());
            // if the current children is missing, then dont bother
            // checking children.
            if !self.element_senders.contains_key(current_path) {
                collector.push(current_path.clone());
                continue;
            }
            self.collect_missing_path_children(
                collector,
                current_path,
                tree,
                &tree.children(node)
            );
            current_path.pop();
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
    highwatermark: usize,
    fatal_error_handle: ShutdownHandle<FatalError>
) -> Result<(), FileReaderError>
where
    S: AsyncRead + Unpin,
{
    let workload_counter_seq = Arc::new(AtomicU8::new(0));
    let mut reader_context: ReaderContext = ReaderContext::new(descriptors.get_root().ok_or(FileReaderError("empty descriptors".into()))?);
    let reader = BufReader::new(src);
    let mut reader = Reader::from_reader(reader);
    let mut read_buf = Vec::new();
    reader.config_mut().trim_text(true);
    loop {
        tokio::select! {
            biased;
            _ = fatal_error_handle.is_cancelled() => {
                drop(reader);
                break;
            }
            event = reader.read_event_into_async(&mut read_buf) => {
                match event {
                    Ok(mut event) => {
                        if handle_reader_event(
                            &mut reader_context,
                            &mut event,
                            file_id,
                            file,
                            &descriptors,
                            sender,
                            collector_sender,
                            workload_counter_seq.clone(),
                            fatal_error_handle.clone(),
                            highwatermark
                        ).await {
                            break;
                        }
                        if let Event::Eof = &event {
                            break;
                        }
                    },
                    Err(err) => {
                        if let Err(err) = collector_sender.send(FileResult::Aborted(file_id, format!("error reading xml : {:?}", err), file.into(), workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1)).await {
                            fatal_error_handle.trigger_fatal(err.into()).await;
                        };
                    }
                }
            }
        }
        read_buf.clear();        
    };
    Ok(())
}

async fn handle_reader_event<'a, 'b>(
    mut reader_context: &'b mut ReaderContext<'a>,
    event: &mut Event<'_>,
    file_id: u64,
    file: &str,
    descriptors: &'a Arc<Tree>, 
    sender: &Sender<XmlWorkload>,
    collector_sender: &Sender<FileResult>,
    workload_counter_seq: Arc<AtomicU8>,
    fatal_error_handle: ShutdownHandle<FatalError>,
    highwatermark: usize,
) -> bool {
    match event {
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
                            collector_sender,
                            workload_counter_seq,
                            fatal_error_handle,
                            highwatermark
                        ).await;
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
                                    collector_sender,
                                    workload_counter_seq,
                                    fatal_error_handle,
                                    highwatermark
                                ).await;
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
                    if let Some(_) = reader_context.current_view.pop() {
                        // we must pop the current path also, to ensure proper tracking
                        reader_context.current_path.pop();
                        // finally, we must restore the parent descriptor as current descriptor
                        reader_context.set_parent_swap_descriptor(&descriptors)
                    }
                }
            }
        },
        Event::Text(text) => {
            // if we match any text, we must add it to the top of the pill view
            let length = reader_context.current_view.len();
            let view_context = &mut reader_context.current_view[length - 1];
             if let Err(err) = view_context.text_sender.send(String::from_utf8_lossy(&text.clone().into_inner()).trim().into()) {
                if let Err(err) =  collector_sender.send(FileResult::Aborted(
                    file_id,
                    format!("error sending node view text : {:?}", err),
                    file.into(),
                    workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1
                )).await {
                    fatal_error_handle.trigger_fatal(err.into()).await;
                }
                return true;
             }
        },
        Event::Eof => {
            match reader_context.missed_test_paths(descriptors) {
                Some(missed_paths) => {
                    // if we are missing some paths, we need to terminate on error
                    if let Err(err) = collector_sender.send(FileResult::Missed(file_id, missed_paths, file.into(), workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1)).await {
                        fatal_error_handle.trigger_fatal(err.into()).await;
                    };
                },
                None => {
                    // we need to send the termination of file to the collector so that it can emit the diagnostic
                    if let Err(err) = collector_sender.send(FileResult::Terminated(file_id, file.into(), workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1)).await {
                        fatal_error_handle.trigger_fatal(err.into()).await;
                    };
                }
            }
        },
        _ => {}
    }
    false
}

async fn handle_path_match<'a, 'b>(
    file_id: u64,
    file: &str,
    workload_id_seq: Arc<AtomicU8>,
    tag: &mut BytesStart<'_>,
    reader_context: &'b mut ReaderContext<'a>,
    sender: &Sender<XmlWorkload>,
    collector_sender: &Sender<FileResult>,
    workload_counter_seq: Arc<AtomicU8>,
    fatal_error_handle: ShutdownHandle<FatalError>,
    highwatermark: usize
) -> bool {
    // if there is no current channel, initiate one.
    // also push a new index to the stack
    // as long as the path
    let path = reader_context.current_descriptor.path().clone();
    reader_context.current_path.push(path);
    if !reader_context.element_senders.contains_key(&reader_context.current_path) {
        // creating the channel
        let (tx, rx) = channel::<Arc<Mutex<FullNodeView>>>(highwatermark);
        //send the worload, including the event receiver
        if let Err(err) = sender.send(
            XmlWorkload {
                file_id,
                workload_counter: workload_id_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                file: file.to_string(),
                path: reader_context.current_path.clone(),
                tag: String::from_utf8_lossy(tag.name().into_inner()).to_string(),
                rules: reader_context.current_descriptor.rules(),
                ctx: reader_context.global_context.clone(),
                events: rx,
            }
        ).await {
            fatal_error_handle.trigger_fatal(err.into()).await;
        };
        // we then store the sender on the hashmap, as long as the current element index
        let path = reader_context.current_path.clone();
        reader_context.element_senders.insert(path, SenderContext{sender: tx, path_index: PathIndex(0)});
    } else {
        // if there is already one, it's a new element of a stream, we need to
        // increment the current index
        if let Some(SenderContext { sender: _, path_index }) = reader_context.element_senders.get_mut(&reader_context.current_path) {
            *path_index += 1
        };
    }
    // in case the element contains a text content,
    // we need to keep it until the end tag is met
    let mut attrs = HashMap::new();
    for attr in tag.attributes() {
        if let Ok(attr) = attr {
            attrs.insert(
                String::from_utf8_lossy(attr.key.as_ref()).into(),
                String::from_utf8_lossy(attr.value.as_ref()).into()
            );
        };
    }
    
    match reader_context.element_senders.get(&reader_context.current_path) {
        Some(SenderContext { sender, path_index }) => {
            let (text_sender, text_receiver) = broadcast::channel(highwatermark);
            let text_receiver = text_receiver;
            let (sender_stack, receiver_stack): (
                Vec<(Vec<Path>, BroadcastSender<PartialNodeView>)>,
                Vec<(Vec<Path>, BroadcastReceiver<PartialNodeView>)>
            ) = reader_context.current_descriptor.map_children().unwrap_or(&Vec::new()).iter()
            .map(|child| {
                let (tx, rx) = broadcast::channel::<PartialNodeView>(highwatermark);
                ((child.clone(), tx), (child.clone(), rx))
            }).unzip();
            // create view
            let inner_view = FullNodeView::new(
                attrs.clone(),
                path_index.into(),
                text_receiver,
                HashMap::from_iter(receiver_stack)
            );
            let inner_view_index = inner_view.index();
            let view = Arc::new(
                Mutex::new(
                    inner_view
                )
            );
            let partial_view = PartialNodeView::new(
                attrs.clone(),
                inner_view_index,
                Arc::new(text_sender.subscribe())
            );
            // if we match the context mapper, we must register the view inside the global context
            if reader_context.current_descriptor.map_view() {
                let mut global_context = reader_context.global_context.lock().await;
                
                global_context.insert(
                    reader_context.current_path.clone(),
                    partial_view
                );
            }
            // TODO : check if any stacked current_view need this view
            let current_path = &reader_context.current_path;
            let children_sender_results= reader_context.current_view.iter()
            .flat_map(|view_context| view_context.children_sender.iter())
            .filter(|(desired_path, _children_sender)| current_path.eq(*desired_path))
            .map(|(_desired_path, children_sender)| children_sender.send(PartialNodeView::new(
                attrs.clone(),
                inner_view_index,
                Arc::new(text_sender.subscribe())
            )));
            for children_sender_result in children_sender_results {
                    if let Err(err) = children_sender_result {
                        println!("determine what to do when the inner children dependency sending fails : {:?}", err);
                    }
            }
            
            // push it to stack
            reader_context.current_view.push(
                CurrentViewContext {
                    text_sender: text_sender,
                    children_sender: HashMap::from_iter(sender_stack)
                }
            );
            
            // send it for processing
            if let Err(err) = sender.send(view).await {
                match collector_sender.send(FileResult::Aborted(
                    file_id,
                    format!("error sending node view : {:?}", err),
                    file.into(),
                    workload_counter_seq.load(std::sync::atomic::Ordering::Relaxed) - 1
                )).await {
                    Ok(()) => {
                        return true;
                    },
                    Err(err) => {
                        fatal_error_handle.trigger_fatal(err.into()).await;
                    }
                };
            }
        },
        None => {
            // cannot happen, since we check synchronously for existance a few lines above
        }
    }
    false
}