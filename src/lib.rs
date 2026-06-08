use std::collections::HashMap;

use crate::nodebuilder::{CutpointStrategy, NodeBuilder, NodeView, Path, Root, Strategy};

pub mod nodebuilder;


fn test() {

    let parent_mapper = |view: NodeView, ctx: &mut HashMap<String, String>| {
        // impl mapping
    };

    let test = Root::new()
    .add_rule(rule)
        .path(
            Path::Child("Invoice".into()),
            Strategy::CutPoint(CutpointStrategy::Group(10)),
            Some(&parent_mapper)
        )
        .add_rule(rule)
            .path(Path::Child("Address".into()), Strategy::Passthrough, None)
            .add_rule(rule)
            .build()
        .add_rule(rule)
        .build()
    .build();
}