use crate::ruleset::{NodeBuilder, Path, Root, Strategy};

pub mod ruleset;


fn test() {

    let test = Root::new()
    .add_rule(rule)
        .path(Path::Child("Invoice".into()), Strategy::Passthrough)
        .add_rule(rule)
            .path(Path::Child("Address".into()), Strategy::Passthrough)
            .add_rule(rule)
            .build()
        .add_rule(rule)
        .build()
    .build();
}