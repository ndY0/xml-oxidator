

# allow to return a list of diagnostics from the workers, as a rule may be individual to a list

    TODO : necessary

# allow to substitue diagnotic infos in failed assertions

    DONE

# add indexes to views

    DONE : usefull to specify a node i a list that failed the assertion

# introduce a mechanism to handle deadlocks when too much worloads start and plug the system waiting for views

    DONE : throws a fatal error instead. this is a configuration issue, and should not allow futher execution.

# implement gracefull fatal error handling

    DONE : implemented using tokio cancellation token
           specifically, if any error sending to structural channels (channels between logical pools) arise,
           a fatal error is returned

# implement aborted file

    DONE : if specific errors arise from reader, concerning the data processing itself.

# Implement the possibility to subscribe to child node

    ROADMAP:
        - convert fold to an FnAsync, so we can manipulate tokio receivers inside
        - convert the NodeView text field from Option<String> to Receiver<String>
        - transform NodeView to Enum, with two flavours :
            - FullNodeView : attr, text receiver and children receivers. text receiver is broadcast
            - PartialNodeView : attr and text receiver only. text receiver is broadcast
        - add a children prop to FullNodeView, wich will be of type HashMap<Vec<Path>, Receiver<PartialNodeView>>. receiver is broadcast
        - adapt reader to send node view immediately, and add push text if receivied, or none at closing tag
        - adapt rule builder to allow adding child subscription for a path. if path does not exists, then receiver will be dropped with no data sent.
        - adapt reader to track child subscriptions in current node stack. at each new node, scan for dependency on the stack

# Implement the parent context

    ROADMAP:
        - change context from HashMap<String, String> to HashMap<Vec<Path>, PartialNodeView>
        - adapt reader to add the binded partial node if descriptor says so
        - adapt rule builder to allow to bind a node partial view to the context
        - finish implementing the context consumption properly

# Implement a check for unused declared rules that should trigger an error

    DONE : implementation backed by the sender hashmap comparaison against the rule tree

# Handle out of order results when file has been aborted

    DONE : abortion or missed events are specific termination events, wich carry additionnal data.
    whith this approach, normal termination mattern can be operated

# test performance and all configs

    TODO

# create a templating engine to simplify declaration

    ROADMAP:
        - learn macros beforehand lol