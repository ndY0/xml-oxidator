use std::collections::HashMap;

pub trait NodeHandler {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: Node);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Rule) -> Self::AddRuleOutput;
    fn path(self, path: Path, strategy: Strategy) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

struct ChildBuilder;
struct RootBuilder;

struct InitState;
struct NodeAdderState;

pub struct Root<State> {
    _state: std::marker::PhantomData<State>,
    strategy: Strategy,
    path: Path,
    nodes: HashMap<Path, Node>,
    rules: Vec<Rule>
}

impl NodeHandler for Root<NodeAdderState> {
    type ToBuilderOuput = Root<InitState>;
    fn add_node(&mut self, path: Path, node: Node) {
        self.nodes.insert(path, node);
    }
    fn to_builder(self) -> Self::ToBuilderOuput {
        Root {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            strategy: self.strategy,
            path: self.path,
            rules: self.rules
        }
    }
}

impl NodeBuilder for Root<InitState> {
    type AddRuleOutput = Root<InitState>;

    type PathOutput = Child<InitState, Root<NodeAdderState>>;

    type BuildOutput = Node;

    fn add_rule(mut self, rule: Rule) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, strategy: Strategy) -> Self::PathOutput {
        let build_parent: Root<NodeAdderState> = Root {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            path: self.path,
            strategy: self.strategy,
            rules: self.rules
        };
        Child::new(build_parent, path, strategy)
    }

    fn build(self) -> Self::BuildOutput {
        Node::new(
            self.strategy,
            self.rules,
            self.nodes
        )
    }
}

impl Root<InitState> {
    pub fn new() -> Self {
        Self {
            _state: std::marker::PhantomData,
            strategy: Strategy::Passthrough,
            path: Path::Root,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

pub struct Child<State, Parent> {
    _state: std::marker::PhantomData<State>,
    strategy: Strategy,
    parent: Parent,
    path: Path,
    nodes: HashMap<Path, Node>,
    rules: Vec<Rule>
}

impl <P: NodeHandler> NodeHandler for Child<NodeAdderState, P> {
    
    type ToBuilderOuput = Child<InitState, P>;

    fn add_node(&mut self, path: Path, node: Node) {
        self.nodes.insert(path, node);
    }
    
    fn to_builder(self) -> Self::ToBuilderOuput {
        Child {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            path: self.path,
            parent: self.parent,
            strategy: self.strategy,
            rules: self.rules
        }
    }
}

impl <P: NodeHandler> Child<NodeAdderState, P> {
    fn to_builder(self) -> Child<InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            parent: self.parent,
            path: self.path,
            rules: self.rules,
            strategy: self.strategy
        }
    }
}

impl <P: NodeHandler> NodeBuilder for Child<InitState, P> {
    type AddRuleOutput = Child<InitState, P>;

    type PathOutput = Child<InitState, Child<NodeAdderState, P>>;

    type BuildOutput = <P as NodeHandler>::ToBuilderOuput;

    fn add_rule(mut self, rule: Rule) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, strategy: Strategy) -> Self::PathOutput
        where Child<NodeAdderState, P>: NodeHandler
    {
        let build_parent = Child {
            _state: std::marker::PhantomData,
            parent: self.parent,
            nodes: self.nodes,
            path: self.path,
            strategy: self.strategy,
            rules: self.rules
        };
        Child::new(build_parent, path, strategy)
    }

    fn build(mut self) -> Self::BuildOutput {
        let current_node = Node::new(
            self.strategy,
            self.rules,
            self.nodes
        );
        self.parent.add_node(self.path, current_node);

        self.parent.to_builder()
    }
}

impl <P: NodeHandler> Child<InitState, P> {
    pub fn new(
        parent: P,
        path: Path,
        strategy: Strategy
    ) -> Child<InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            strategy,
            path,
            parent: parent,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}


pub struct Node {
    strategy: Strategy,
    rules: Vec<Rule>,
    nodes: HashMap<Path, Node>,
}

impl Node {
    pub fn new(
        strategy: Strategy,
        rules: Vec<Rule>,
        nodes: HashMap<Path, Node>
    ) -> Self {
        Self {
            strategy,
            rules,
            nodes
        }
    }
}

struct Rule {
    matcher: Matcher,
    test: Box<dyn Fn(NodeView) -> bool>,
    assertion: String
}

pub struct NodeView {

}

pub enum Matcher {

}

pub enum Strategy {
    Passthrough,

}

#[derive(PartialEq, Eq, Hash)]
pub enum Path {
    Root,
    Child(String)
}

impl ToString for Path {
    fn to_string(&self) -> String {
        match self {
            Path::Child(path) => {
                path.clone()
            },
            Path::Root => "/".into()
        }
    }
}