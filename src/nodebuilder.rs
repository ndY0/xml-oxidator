use std::collections::HashMap;

pub trait NodeHandler<'a> {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: Node<'a>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder<'a, NodeView> {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Rule<NodeView>) -> Self::AddRuleOutput;
    fn path(self, path: Path, strategy: Strategy, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

struct ChildBuilder;
struct RootBuilder;

struct InitState;
struct NodeAdderState;

pub struct Root<'a, State> {
    _state: std::marker::PhantomData<State>,
    strategy: Strategy,
    path: Path,
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Rule<SingleNodeView>>
}

impl <'a> NodeHandler<'a> for Root<'a, NodeAdderState> {
    type ToBuilderOuput = Root<'a, InitState>;
    fn add_node(&mut self, path: Path, node: Node<'a>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) {
        self.nodes.insert(path, (node, map_from_parent));
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

impl <'a> NodeBuilder<'a> for Root<'a, InitState> {
    type AddRuleOutput = Root<'a, InitState>;

    type PathOutput = Child<'a, InitState, Root<'a, NodeAdderState>>;

    type BuildOutput = Node<'a>;

    fn add_rule(mut self, rule: Rule) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, strategy: Strategy, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput {
        let build_parent: Root<NodeAdderState> = Root {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            path: self.path,
            strategy: self.strategy,
            rules: self.rules
        };
        Child::new(build_parent, path, strategy, map_from_parent)
    }

    fn build(self) -> Self::BuildOutput {
        Node::new(
            self.strategy,
            self.rules,
            self.nodes
        )
    }
}

impl <'a> Root<'a, InitState> {
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

pub struct Child<'a, State, Parent, NodeView> {
    _state: std::marker::PhantomData<State>,
    strategy: Strategy,
    map_from_parent: Option<&'a ParentPropertyMapper<'a>>,
    parent: Parent,
    path: Path,
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Rule<NodeView>>
}

impl <'a, P: NodeHandler<'a>> NodeHandler<'a> for Child<'a, NodeAdderState, P> {
    
    type ToBuilderOuput = Child<'a, InitState, P>;

    fn add_node(&mut self, path: Path, node: Node<'a>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) {
        self.nodes.insert(path, (node, map_from_parent));
    }
    
    fn to_builder(self) -> Self::ToBuilderOuput {
        Child {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            map_from_parent: self.map_from_parent,
            path: self.path,
            parent: self.parent,
            strategy: self.strategy,
            rules: self.rules
        }
    }
}

impl <'a, P: NodeHandler<'a>> Child<'a, NodeAdderState, P> {
    fn to_builder(self) -> Child<'a, InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            map_from_parent: self.map_from_parent,
            parent: self.parent,
            path: self.path,
            rules: self.rules,
            strategy: self.strategy
        }
    }
}

impl <'a, P: NodeHandler<'a>> NodeBuilder<'a> for Child<'a, InitState, P> {
    type AddRuleOutput = Child<'a, InitState, P>;

    type PathOutput = Child<'a, InitState, Child<'a, NodeAdderState, P>>;

    type BuildOutput = <P as NodeHandler<'a>>::ToBuilderOuput;

    fn add_rule(mut self, rule: Rule) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, strategy: Strategy, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput
        where Child<'a, NodeAdderState, P>: NodeHandler<'a>
    {
        let build_parent = Child {
            _state: std::marker::PhantomData,
            map_from_parent: self.map_from_parent,
            parent: self.parent,
            nodes: self.nodes,
            path: self.path,
            strategy: self.strategy,
            rules: self.rules
        };
        Child::new(build_parent, path, strategy, map_from_parent)
    }

    fn build(mut self) -> Self::BuildOutput {
        let current_node = Node::new(
            self.strategy,
            self.rules,
            self.nodes
        );
        self.parent.add_node(self.path, current_node, self.map_from_parent);

        self.parent.to_builder()
    }
}

impl <'a, P: NodeHandler<'a>> Child<'a, InitState, P> {
    pub fn new(
        parent: P,
        path: Path,
        strategy: Strategy,
        map_from_parent: Option<&'a ParentPropertyMapper<'a>>
    ) -> Child<'a, InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            strategy,
            path,
            map_from_parent,
            parent,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

pub type ParentPropertyMapper<'a> = dyn Fn(NodeView, &'a mut HashMap<String, String>);

pub struct Node<'a, NodeView> {
    strategy: Strategy,
    rules: Vec<Rule<NodeView>>,
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,

}

impl <'a, NodeView> Node<'a, NodeView> {
    pub fn new(
        strategy: Strategy,
        rules: Vec<Rule<NodeView>>,
        nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>
    ) -> Self {
        Self {
            strategy,
            rules,
            nodes
        }
    }
}

struct Rule<NodeView> {
    test: Box<dyn Fn(NodeView, HashMap<String, String>) -> bool>,
    assertion: String
}

pub struct SingleNodeView {
    text: Option<String>,
    attrs: HashMap<String, String>
}

impl SingleNodeView {
    fn text(&self) -> Option<&String> {
        self.text.as_ref()
    }
    fn attr(&self, key: &str) -> Option<&String> {
        self.attrs.get(key)
    }
}

pub struct ListNodeView {
    from: usize,
    to: usize,
    views: Vec<SingleNodeView>
}

pub enum Strategy {
    Passthrough,
    CutPoint(CutpointStrategy),
}

pub enum CutpointStrategy {
    Unit,
    Group(u32)
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