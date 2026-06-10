use std::{collections::HashMap, sync::Arc};

use crate::rulebuilder::{NodeBuilder, NodeView, Path, Root, RuleBuilder};

pub mod rulebuilder;
pub mod filereader;

fn test() {

    let parent_mapper = |view: &NodeView, ctx: &mut HashMap<String, String>| {
        // impl mapping
    };
    let test = "un-id";
    let rule_builder = RuleBuilder::test(Arc::new(|view, ctx| {
                view.attr("UUID").map_or(false, |id| {id.eq(test)})
            }))
            .fold(Arc::new(|acc, curr| {
                *acc || curr
            }))
            .init(Box::new(|| { false }))
            .assert(Arc::new(|res| {*res}));

    let test = Root::new("Invoices")
    .add_rule(
        rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
    )
        .path(
            Path("Invoice".into()),
            Some(&parent_mapper)
        )
        .add_rule(
            rule_builder.build("nous rencontrons un pb de check sur l'identifiant")
        )
            .path(Path("Address".into()), None)
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