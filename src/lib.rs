use std::{collections::HashMap, sync::Arc};

use crate::nodebuilder::{NodeBuilder, NodeView, Path, Root, RuleBuilder};

pub mod nodebuilder;

fn test() {

    let parent_mapper = |view: &NodeView, ctx: &mut HashMap<String, String>| {
        // impl mapping
    };

    let rule_builder = RuleBuilder::test(Arc::new(|view, ctx| {
                view.attr("UUID").map_or(false, |id| {id.eq("un id")})
            }))
            .fold(Arc::new(|acc, curr| {
                acc || curr
            }))
            .init(Arc::new(|| { false }))
            .assert(Arc::new(|res| {*res}));

    let test = Root::new()
    .add_rule(
        rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
    )
        .path(
            Path::Child("Invoice".into()),
            Some(&parent_mapper)
        )
        .add_rule(
            rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
        )
            .path(Path::Child("Address".into()), None)
            .add_rule(
                rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
            )
            .build()
        .add_rule(
            rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
        )
        .build()
    .build();
}