use std::{sync::Arc, time::Duration};
use tokio::sync::mpsc::channel;
use tokio_test::io::{Builder, Mock};

use xml_oxydizer::{init::{FileInfo, start}, rulebuilder::{NodeBuilder, Root, RuleBuilder, Tree}};

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
        .build("root \"test\" attribute is not of value \"value\"")
    )
    .build().unwrap()
}

fn build_simple_async_xml_reader() -> Mock {
    Builder::new()
    .read(b"<root test=\"value\">content root<child test2=\"value2\">")
    .wait(Duration::from_micros(10))
    .read(b"<child/></root>")
    .build()
}

#[tokio::test]
async fn test_small_simple_file() {

    let (file_sender, file_receiver) = channel::<FileInfo<Mock>>(1);
    let (error_sender, mut error_receiver) = channel(1);
    let (diagnostic_sender, mut diagnostic_receiver) = channel(1);

    let descriptors = Arc::new(build_simple_descriptors());

    let test_handler = tokio::spawn(async move {
        match start(
            file_receiver,
            &error_sender,
            &diagnostic_sender,
            1,
            2,
            10,
            10,
            2
        ).await {
            Ok(_) => {
    
            },
            Err(err) => {}
        };
    });
    // we send the file
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
    match error_receiver.recv().await {
        Some(data) => {
            println!("received data : {:?}", data);
        },
        None => {

        }
    }

}