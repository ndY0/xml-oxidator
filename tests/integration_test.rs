
#[cfg(test)]
mod tests {

    use std::{collections::HashMap, sync::Arc};
use tokio::{spawn, sync::{Mutex, mpsc::channel}};
    use tokio_test::io::{Builder, Mock};
    use std::time::Instant;
    #[cfg(feature = "test-heap")]
    use dhat::Alloc;
    
    use xml_oxydizer::{init::{FileInfo, start}, rulebuilder::{CommonNodeView, FullNodeView, NodeBuilder, PartialNodeView, Path, Root, RuleBuilder, Tree}};

    #[cfg(feature = "test-heap")]
    #[global_allocator]
    static ALLOC: Alloc = Alloc;

    fn build_simple_descriptors() -> Tree {

        let test_root = |view: Arc<Mutex<FullNodeView>> , _ctx| async move {
            let mut view = view.lock().await;
            let is_attr_ok = view.attr("test").is_some_and(|value| {value == "value"});
            let is_child_attr_ok: bool = match view.children().get_mut(&vec![Path("root".into()), Path("child".into())]) {
                    Some(child_receiver) => {
                        let mut child_attr_ok = true;
                        loop {
                            child_attr_ok = child_attr_ok && match child_receiver.recv().await {
                                Ok(view) => {
                                    if let Some(attr) = view.attr("test2") {
                                        attr == "value2"
                                    } else {
                                        false
                                    }
                                },
                                Err(_err) => false
                            }
                        }
                    },
                    None => false
                };
            is_attr_ok && is_child_attr_ok
        };

        let test_child = |view: Arc<Mutex<FullNodeView>> , ctx: Arc<Mutex<HashMap<Vec<Path>, PartialNodeView>>>| async move {
            let view = view.lock().await;
            let is_attr_ok = view.attr("test").is_some_and(|value| {value == "value"});
            let ctx = ctx.lock().await;
            let is_paren_ok = match ctx.get(&vec![Path("child".into())]) {
                Some(view) => {
                    if let Some(attr) = view.attr("test") {
                        attr == "value2"
                    } else {
                        false
                    }
                },
                None => false
            };
            is_attr_ok && is_paren_ok
        };

        Root::new("root", true, Some(vec![vec![Path("root".into()), Path("child".into())]]))
        .add_rule(
            RuleBuilder::test(
                "test_rule".into(),
                test_root
            )
            .fold(|acc, curr| {*acc || curr})
            .init(|| {false})
            .assert(|acc| {*acc})
            .build("root \"test\" attribute is not of value \"value\", rule=[{rule}], path=[{path}]")
        )
        .path(Path("child".into()), false, None)
            .add_rule(
                RuleBuilder::test(
                    "test_child".into(),
                    test_child
                )
                .fold(|acc, curr| {*acc || curr})
                .init(|| {false})
                .assert(|acc| {*acc})
                .build("root \"test2\" attribute is not of value \"value2\", rule=[{rule}], path=[{path}]")
            )
            .build()
        .build()
        .unwrap()
    }
    
    fn build_simple_async_xml_reader() -> Mock {
        let mut builder = Builder::new();
        builder
        .read(b"<root test=\"vaue\">content root");
    
        for _ in 1..=10_000 {
            builder
            .read(b"<child test2=\"value2\">")
            .read(b"</child>");
        }
        builder.read(b"</root>");
        builder.build()
    }
    
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn test_small_simple_file() {

        #[cfg(feature = "test-heap")]
        let _profiler = dhat::Profiler::new_heap();
    
        let (file_sender, file_receiver) = channel::<FileInfo<Mock>>(1);
        let (diagnostic_sender, mut diagnostic_receiver) = channel(1);
    
        let descriptors = Arc::new(build_simple_descriptors());
    
        let test_handler = tokio::spawn(async move {
            match start(
                file_receiver,
                &diagnostic_sender,
                1,
                10,
                2,
                10,
                10
            ).await {
                Ok(_) => {
        
                },
                Err(err) => {
    
                }
            };
        });
        // start of the actual payload
        spawn(async move {
            match file_sender.send(
                FileInfo::new(
                    "test.xml",
                    Arc::clone(&descriptors),
                    Box::new(|| {
                        build_simple_async_xml_reader()
                    })
                )
            ).await {
                Ok(_) => {
        
                },
                Err(err) => {
        
                }
            };
        });
        let start = Instant::now();
        // we send the file
        match diagnostic_receiver.recv().await {
            Some(data) => {
                println!("received data : {:?}", data);
            },
            None => {
    
            }
        }
        // end of treatment
        println!("test duration : {:?}", start.elapsed())
    
    }

}
