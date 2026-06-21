
#[cfg(test)]
mod tests {

    use std::{sync::Arc};
    use tokio::{spawn, sync::mpsc::channel};
    use tokio_test::io::{Builder, Mock};
    use std::time::Instant;
    #[cfg(feature = "test-heap")]
    use dhat::Alloc;
    
    use xml_oxydizer::{init::{FileInfo, start}, rulebuilder::{NodeBuilder, Path, Root, RuleBuilder, Tree}};

    #[cfg(feature = "test-heap")]
    #[global_allocator]
    static ALLOC: Alloc = Alloc;

    fn build_simple_descriptors() -> Tree {
        Root::new("root")
        .add_rule(
            RuleBuilder::test(
                "test_rule".into(),
                Arc::new(|view, _ctx| { view.attr("test").is_some_and(|value| {value == "value"}) })
            )
            .fold(Arc::new(|acc, curr| {*acc || curr}))
            .init(Box::new(|| {false}))
            .assert(Arc::new(|acc| {*acc}))
            .build("root \"test\" attribute is not of value \"value\", rule=[{rule}], path=[{path}]")
        )
        .path(Path("child".into()), None)
            .add_rule(
                RuleBuilder::test(
                    "test_child".into(),
                    Arc::new(|view, _ctx| { view.attr("test2").is_some_and(|value| {value == "value2"}) })
                )
                .fold(Arc::new(|acc, curr| {*acc || curr}))
                .init(Box::new(|| {false}))
                .assert(Arc::new(|acc| {*acc}))
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

    fn build_simple_descriptors_depeer() -> Tree {
        Root::new("root")
        .add_rule(
            RuleBuilder::test(
                "test_rule".into(),
                Arc::new(|view, _ctx| { view.attr("test").is_some_and(|value| {value == "value"}) })
            )
            .fold(Arc::new(|acc, curr| {*acc || curr}))
            .init(Box::new(|| {false}))
            .assert(Arc::new(|acc| {*acc}))
            .build("root \"test\" attribute is not of value \"value\", rule=[{rule}], path=[{path}]")
        )
        .path(Path("child".into()), None)
            .add_rule(
                RuleBuilder::test(
                    "test_child".into(),
                    Arc::new(|view, _ctx| { view.attr("test2").is_some_and(|value| {value == "value2"}) })
                )
                .fold(Arc::new(|acc, curr| {*acc || curr}))
                .init(Box::new(|| {false}))
                .assert(Arc::new(|acc| {*acc}))
                .build("child \"test2\" attribute is not of value \"value2\", rule=[{rule}], path=[{path}]")
            )
            .path(Path("nested".into()), None)
                .add_rule(
                    RuleBuilder::test(
                        "test_nested".into(),
                        Arc::new(|view, _ctx| { view.attr("test2").is_some_and(|value| {value == "value2"}) })
                    )
                    .fold(Arc::new(|acc, curr| {*acc || curr}))
                    .init(Box::new(|| {false}))
                    .assert(Arc::new(|acc| {*acc}))
                    .build("nested \"test2\" attribute is not of value \"value2\", rule=[{rule}], path=[{path}]")
                )
                .path(Path("nested".into()), None)
                    .add_rule(
                        RuleBuilder::test(
                            "test_nested".into(),
                            Arc::new(|view, _ctx| { view.attr("test2").is_some_and(|value| {value == "value2"}) })
                        )
                        .fold(Arc::new(|acc, curr| {*acc || curr}))
                        .init(Box::new(|| {false}))
                        .assert(Arc::new(|acc| {*acc}))
                        .build("nested \"test2\" attribute is not of value \"value2\", rule=[{rule}], path=[{path}]")
                    )
                    .path(Path("nested".into()), None)
                        .add_rule(
                            RuleBuilder::test(
                                "test_nested".into(),
                                Arc::new(|view, _ctx| { view.attr("test2").is_some_and(|value| {value == "value2"}) })
                            )
                            .fold(Arc::new(|acc, curr| {*acc || curr}))
                            .init(Box::new(|| {false}))
                            .assert(Arc::new(|acc| {*acc}))
                            .build("nested \"test2\" attribute is not of value \"value2\", rule=[{rule}], path=[{path}]")
                        )
                        .build()
                    .build()
                .build()
            .build()
        .build()
        .unwrap()
    }

    fn build_simple_nested_async_xml_reader() -> Mock {
        let mut builder = Builder::new();
        builder
        .read(b"<root test=\"vaue\">content root");
    
        for _ in 1..=100_000 {
            builder
            .read(b"<child test2=\"value2\">")
            .read(b"<nested test3=\"value3\">")
            .read(b"<nested test4=\"value4\">")
            .read(b"<nested test5=\"value5\">")
            .read(b"</nested>")
            .read(b"</nested>")
            .read(b"</nested>")
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn test_small_simple_file_deadlock() {

        #[cfg(feature = "test-heap")]
        let _profiler = dhat::Profiler::new_heap();
    
        let (file_sender, file_receiver) = channel::<FileInfo<Mock>>(1);
        let (diagnostic_sender, mut diagnostic_receiver) = channel(1);
    
        let descriptors = Arc::new(build_simple_descriptors_depeer());
    
        let test_handler = tokio::spawn(async move {
            match start(
                file_receiver,
                &diagnostic_sender,
                3,
            10,
                2,
                2,
                10
            ).await {
                Ok(_) => {
        
                },
                Err(err) => {
    
                }
            };
        });
        // start of the actual payload
        let sender = Arc::new(file_sender);
        for _ in 1..=3 {
            let sender = Arc::clone(&sender);
            let descriptors = Arc::clone(&descriptors);
            spawn(async move {
                match sender.send(
                    FileInfo::new(
                        "test.xml",
                        Arc::clone(&descriptors),
                        Box::new(|| {
                            build_simple_nested_async_xml_reader()
                        })
                    )
                ).await {
                    Ok(_) => {
            
                    },
                    Err(err) => {
            
                    }
                };
            });
        }
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
