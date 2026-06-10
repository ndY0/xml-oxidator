use std::{cell::RefCell, collections::HashMap, pin::Pin, rc::{Rc, Weak}, sync::Arc};

use futures::{Stream, StreamExt};

pub trait NodeHandler<'a> {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: Rc<RefCell<Node<'a>>>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder<'a> {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Box<dyn Rule>) -> Self::AddRuleOutput;
    fn path(self, path: Path, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

pub trait Rule: Send {
    fn fold(&mut self, view: &NodeView, ctx: &HashMap<String, String>);
    fn assert(&self) -> Diagnostic;
}

pub struct InitState;
pub struct NodeAdderState;

pub struct Root<'a, State> {
    _state: std::marker::PhantomData<State>,
    path: Path,
    nodes: HashMap<Path, (Rc<RefCell<Node<'a>>>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Box<dyn Rule>>
}

impl <'a> NodeHandler<'a> for Root<'a, NodeAdderState> {
    type ToBuilderOuput = Root<'a, InitState>;
    fn add_node(&mut self, path: Path, node: Rc<RefCell<Node<'a>>>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) {
        self.nodes.insert(path, (node, map_from_parent));
    }
    fn to_builder(self) -> Self::ToBuilderOuput {
        Root {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        }
    }
}

impl <'a> NodeBuilder<'a> for Root<'a, InitState> {
    type AddRuleOutput = Root<'a, InitState>;

    type PathOutput = Child<'a, InitState, Root<'a, NodeAdderState>>;

    type BuildOutput = Rc<RefCell<Node<'a>>>;

    fn add_rule(mut self, rule: Box<dyn Rule>) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput {
        let build_parent: Root<NodeAdderState> = Root {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        };
        Child::new(build_parent, path, map_from_parent)
    }

    fn build(self) -> Self::BuildOutput {

        let parent = Rc::new(
            RefCell::new(
                Node::new(
                    self.path,
                    self.rules
                )
            )
        );
        for (_1, (node, _2)) in &self.nodes {
            node.borrow_mut().set_parent(&parent);
        }
        parent.borrow_mut().set_nodes(self.nodes);
        parent
    }
}

impl <'a> Root<'a, InitState> {
    pub fn new(root: &str) -> Self {
        Self {
            _state: std::marker::PhantomData,
            path: Path(root.into()),
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

pub struct Child<'a, State, Parent> {
    _state: std::marker::PhantomData<State>,
    map_from_parent: Option<&'a ParentPropertyMapper<'a>>,
    parent: Parent,
    path: Path,
    nodes: HashMap<Path, (Rc<RefCell<Node<'a>>>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Box<dyn Rule>>
}

impl <'a, P: NodeHandler<'a>> NodeHandler<'a> for Child<'a, NodeAdderState, P> {
    
    type ToBuilderOuput = Child<'a, InitState, P>;

    fn add_node(&mut self, path: Path, node: Rc<RefCell<Node<'a>>>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) {
        self.nodes.insert(path, (node, map_from_parent));
    }
    
    fn to_builder(self) -> Self::ToBuilderOuput {
        Child {
            _state: std::marker::PhantomData,
            nodes: self.nodes,
            map_from_parent: self.map_from_parent,
            path: self.path,
            parent: self.parent,
            rules: self.rules
        }
    }
}

impl <'a, P: NodeHandler<'a>> NodeBuilder<'a> for Child<'a, InitState, P> {
    type AddRuleOutput = Child<'a, InitState, P>;

    type PathOutput = Child<'a, InitState, Child<'a, NodeAdderState, P>>;

    type BuildOutput = <P as NodeHandler<'a>>::ToBuilderOuput;

    fn add_rule(mut self, rule: Box<dyn Rule>) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput
        where Child<'a, NodeAdderState, P>: NodeHandler<'a>
    {
        let build_parent = Child {
            _state: std::marker::PhantomData,
            map_from_parent: self.map_from_parent,
            parent: self.parent,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        };
        Child::new(build_parent, path, map_from_parent)
    }

    fn build(mut self) -> Self::BuildOutput {
        let parent = Rc::new(
            RefCell::new(
                Node::new(
                    self.path.clone(),
                    self.rules
                )
            )
        );
        for (_1, (node, _2)) in &self.nodes {
            node.borrow_mut().set_parent(&parent);
        }
        parent.borrow_mut().set_nodes(self.nodes);
        self.parent.add_node(self.path, parent, self.map_from_parent);

        self.parent.to_builder()
    }
}

impl <'a, P: NodeHandler<'a>> Child<'a, InitState, P> {
    pub fn new(
        parent: P,
        path: Path,
        map_from_parent: Option<&'a ParentPropertyMapper<'a>>
    ) -> Child<'a, InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            path,
            map_from_parent,
            parent,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

pub type ParentPropertyMapper<'a> = dyn Fn(&NodeView, &'a mut HashMap<String, String>);

pub struct Node<'a> {
    path: Path,
    rules: Vec<Box<dyn Rule>>,
    nodes: HashMap<Path, (Rc<RefCell<Node<'a>>>, Option<&'a ParentPropertyMapper<'a>>)>,
    parent: Weak<RefCell<Node<'a>>>

}

impl <'a> Node<'a> {
    pub fn new(
        path: Path,
        rules: Vec<Box<dyn Rule>>,
        // nodes: HashMap<Path, (Rc<Node<'a>>, Option<&'a ParentPropertyMapper<'a>>)>,
    ) -> Self {
        Self {
            path,
            rules,
            nodes: HashMap::default(),
            parent: Weak::default()
        }
    }
    pub fn set_nodes(&mut self, nodes: HashMap<Path, (Rc<RefCell<Node<'a>>>, Option<&'a ParentPropertyMapper<'a>>)>) {
        self.nodes = nodes;
    }
    pub fn set_parent(&mut self, parent: &Rc<RefCell<Node<'a>>>) {
        self.parent = Rc::downgrade(parent)
    }

    pub fn path(&self) -> &String {
        &self.path.0
    }

    pub fn children(&self) -> &HashMap<Path, (Rc<RefCell<Node<'a>>>, Option<&'a ParentPropertyMapper<'a>>)> {
        &self.nodes
    }
    pub fn parent(&self) -> &Weak<RefCell<Node<'a>>> {
        &self.parent
    }
    pub async fn run(&mut self, mut stream: Pin<Box<dyn Stream<Item = NodeView>>>, ctx: &HashMap<String, String>) -> Vec<RuleResult> {

            while let Some(view) = stream.next().await {
                for rule in self.rules.iter_mut() {
                    rule.fold(&view, ctx);
                }
            }
            self.rules.iter()
            .map(|rule| {
                let diagnostic = rule.assert();
                RuleResult(self.path.clone(), diagnostic.statut, diagnostic.assertion)
            }).collect()
        }
}

pub struct RuleResult(pub Path, pub bool, pub String);

pub struct NoTest;
pub struct NoFold;
pub struct NoInit;
pub struct NoAssert;

pub struct RuleBuilder<
    TestType,
    FoldType,
    InitType,
    AssertType,
> {
    test: TestType,
    fold: FoldType,
    init: InitType,
    assert: AssertType,
}
impl RuleBuilder<
    NoTest,
    NoFold,
    NoInit,
    NoAssert,
> {
    pub fn test<R>(test: Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
        NoFold,
        NoInit,
        NoAssert,
    > {
        RuleBuilder {
            test: test,
            fold: NoFold,
            init: NoInit,
            assert: NoAssert,
        }
    }
}

impl <R: 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    NoFold,
    NoInit,
    NoAssert,
> {
    pub fn fold<Acc: Send + 'static>(self, fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        NoInit,
        NoAssert,
    > {
        RuleBuilder {
            test: self.test,
            fold: fold,
            init: NoInit,
            assert: NoAssert,
        }
    }
}

impl <R: 'static, Acc: Send + 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    NoInit,
    NoAssert,
> {
    pub fn init(self, init: Box<dyn Fn() -> Acc>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        Box<dyn Fn() -> Acc>,
        NoAssert,
    > {
        RuleBuilder {
            test: self.test,
            fold: self.fold,
            init: init,
            assert: NoAssert,
        }
    }
}

impl <R: 'static, Acc: Send + 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    NoAssert,
> {
    pub fn assert(self, assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        Box<dyn Fn() -> Acc>,
        Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
    > {
        RuleBuilder {
            test: self.test,
            fold: self.fold,
            init: self.init,
            assert: assert,
        }
    }
}

impl <R: 'static, Acc: Send + 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
> {
    pub fn build(&self, assertion: &str) -> Box<dyn Rule> {
        Box::new(
            ConcreteRule::new(
                (self.init)(),
                Arc::clone(&self.test),
                Arc::clone(&self.fold),
                Arc::clone(&self.assert),
                assertion.into()
            )
        )
    }
}

pub struct ConcreteRule<Acc: Send + 'static, R: 'static> {
    state: Acc,
    test: Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
    assertion: String
}

impl <Acc: Send + 'static, R: 'static> ConcreteRule<Acc, R> {
    pub fn new(
        init: Acc,
        test: Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
        fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
        assertion: String, 
    ) -> Self {
        Self {
            state: init,
            test,
            fold,
            assert,
            assertion
        }
    }
}

impl <Acc: Send + 'static, R: 'static> Rule for ConcreteRule<Acc, R> {

    fn fold(&mut self, view: &NodeView, ctx: &HashMap<String, String>) {
        self.state = (self.fold)(&self.state, (self.test)(view, ctx))
    }

    fn assert(&self) -> Diagnostic {
        Diagnostic {
            assertion: self.assertion.clone(),
            statut: (self.assert)(&self.state)
        }
    }
}

pub struct Diagnostic {
    pub assertion: String,
    pub statut: bool
}

pub struct NodeView {
    text: Option<String>,
    attrs: HashMap<String, String>
}

impl NodeView {
    pub fn text(&self) -> Option<&String> {
        self.text.as_ref()
    }
    pub fn attr(&self, key: &str) -> Option<&String> {
        self.attrs.get(key)
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Path(pub String);
// pub enum Path {
//     Root,
//     Child(String)
// }

impl ToString for Path {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}