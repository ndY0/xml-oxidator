use std::{cell::RefCell, collections::HashMap, fmt::Display, rc::Rc, sync::Arc};

#[derive(Debug)]
pub struct BuilderError(String);

impl Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}

pub trait NodeHandler {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: usize, map_from_parent: Option<Box<dyn PropertyMapper>>);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Box<dyn Rule>) -> Self::AddRuleOutput;
    fn path(self, path: Path, map_from_parent: Option<Box<dyn PropertyMapper>>) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

// pub type PropertyMapper = Box<dyn Fn(&NodeView, &mut HashMap<String, String>)-> () + Send + Sync>;

pub trait PropertyMapper
where
    Self: Sync + Send + DynClonePropertyMapper
{
    fn map(&self, view: &NodeView, ctx: &mut HashMap<String, String>);
}

// intermediate trait, necessary for the blanket impl
pub trait DynClonePropertyMapper {
    fn clone_box(&self) -> Box<dyn PropertyMapper>;
}

// blanket impl of the intermediate trait
// it is important to restrict to Clone
impl<T: PropertyMapper + Clone + 'static> DynClonePropertyMapper for T {
    fn clone_box(&self) -> Box<dyn PropertyMapper> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn PropertyMapper> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

pub trait Rule
    where Self: Sync + Send + DynCloneRule 
{
    fn fold(&mut self, view: &NodeView, ctx: &HashMap<String, String>);
    fn assert(&self) -> Diagnostic;
}

// intermediate trait, necessary for the blanket impl
pub trait DynCloneRule {
    fn clone_box(&self) -> Box<dyn Rule>;
}

// blanket impl of the intermediate trait
// it is important to restrict to Clone
impl<T: Rule + Clone + 'static> DynCloneRule for T {
    fn clone_box(&self) -> Box<dyn Rule> {
        Box::new(self.clone())
    }
}

impl Clone for Box<dyn Rule> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

pub struct InitState;
pub struct NodeAdderState;

pub struct Root<State> {
    _state: std::marker::PhantomData<State>,
    tree: Rc<RefCell<Tree>>,
    path: Path,
    nodes: HashMap<Path, (usize, Option<Box<dyn PropertyMapper>>)>,
    rules: Vec<Box<dyn Rule>>
}

impl <'a> NodeHandler for Root<NodeAdderState> {
    type ToBuilderOuput = Root<InitState>;
    fn add_node(&mut self, path: Path, node: usize, map_from_parent: Option<Box<dyn PropertyMapper>>) {
        self.nodes.insert(path, (node, map_from_parent));
    }
    fn to_builder(self) -> Self::ToBuilderOuput {
        Root {
            _state: std::marker::PhantomData,
            tree: self.tree,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        }
    }
}

impl NodeBuilder for Root<InitState> {
    type AddRuleOutput = Root<InitState>;

    type PathOutput = Child<InitState, Root<NodeAdderState>>;

    type BuildOutput = Result<Tree, BuilderError>;

    fn add_rule(mut self, rule: Box<dyn Rule>) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, map_from_parent: Option<Box<dyn PropertyMapper>>) -> Self::PathOutput {
        let build_parent: Root<NodeAdderState> = Root {
            _state: std::marker::PhantomData,
            tree: Rc::clone(&self.tree),
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        };
        Child::new(build_parent, Rc::clone(&self.tree), path, map_from_parent)
    }

    fn build(self) -> Self::BuildOutput {

        // we create the parent node
        let mut parent = Node::new(
            self.path,
            self.rules
        );
        match Rc::try_unwrap(self.tree) {
            Ok(tree) => {
                let mut tree = tree.into_inner();
                // for every child it declare, we must set it's parent with the help of the tree
                let parent_index = tree.next_index();
                for (_1, (node, _2)) in &self.nodes {
                    tree.set_child_parent(*node, parent_index);
                }
                parent.set_nodes(self.nodes);
                // we add the node to the vector
                tree.add_node(parent);
                Ok(tree)
            },
            Err(_) => {
                Err(BuilderError("couldn't retrieve the shared tree under construction".into()))
            }
        }
    }
}

impl Root<InitState> {
    pub fn new(root: &str) -> Self {
        Self {
            _state: std::marker::PhantomData,
            tree: Rc::new(RefCell::new(Tree::new())),
            path: Path(root.into()),
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

pub struct Child<State, Parent> {
    _state: std::marker::PhantomData<State>,
    tree: Rc<RefCell<Tree>>,
    map_from_parent: Option<Box<dyn PropertyMapper>>,
    parent: Parent,
    path: Path,
    nodes: HashMap<Path, (usize, Option<Box<dyn PropertyMapper>>)>,
    rules: Vec<Box<dyn Rule>>
}

impl <P: NodeHandler> NodeHandler for Child<NodeAdderState, P> {
    
    type ToBuilderOuput = Child<InitState, P>;

    fn add_node(&mut self, path: Path, node: usize, map_from_parent: Option<Box<dyn PropertyMapper>>) {
        self.nodes.insert(path, (node, map_from_parent));
    }
    
    fn to_builder(self) -> Self::ToBuilderOuput {
        Child {
            _state: std::marker::PhantomData,
            tree: self.tree,
            nodes: self.nodes,
            map_from_parent: self.map_from_parent,
            path: self.path,
            parent: self.parent,
            rules: self.rules
        }
    }
}

impl <P: NodeHandler> NodeBuilder for Child<InitState, P> {
    type AddRuleOutput = Child<InitState, P>;

    type PathOutput = Child<InitState, Child<NodeAdderState, P>>;

    type BuildOutput = <P as NodeHandler>::ToBuilderOuput;

    fn add_rule(mut self, rule: Box<dyn Rule>) -> Self::AddRuleOutput {
        self.rules.push(rule);
        self
    }

    fn path(self, path: Path, map_from_parent: Option<Box<dyn PropertyMapper>>) -> Self::PathOutput
        where Child<NodeAdderState, P>: NodeHandler
    {
        let build_parent = Child {
            _state: std::marker::PhantomData,
            tree: Rc::clone(&self.tree),
            map_from_parent: self.map_from_parent,
            parent: self.parent,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        };
        Child::new(build_parent, Rc::clone(&self.tree), path, map_from_parent)
    }

    fn build(mut self) -> Self::BuildOutput {
        let mut parent = Node::new(
            self.path.clone(),
            self.rules
        );
        let mut tree = self.tree.borrow_mut();
        // for every child it declare, we must set it's parent with the help of the tree
        let parent_index = tree.next_index();
        for (_1, (node, _2)) in &self.nodes {
            tree.set_child_parent(*node, parent_index);
        }
        parent.set_nodes(self.nodes);
        // we add the node to the vector
        tree.add_node(parent);

        self.parent.add_node(self.path, parent_index, self.map_from_parent);

        self.parent.to_builder()
    }
}

impl <P: NodeHandler> Child<InitState, P> {
    pub fn new(
        parent: P,
        tree: Rc<RefCell<Tree>>,
        path: Path,
        map_from_parent: Option<Box<dyn PropertyMapper>>
    ) -> Child<InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            tree,
            path,
            map_from_parent,
            parent,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

#[derive(Clone)]
pub struct Tree {
    pub nodes: Vec<Node>
}

impl Tree {

    pub fn new() -> Self {
        Self {
            nodes: Vec::new()
        }
    }

    pub fn get_root(&self) -> Option<&Node> {
        self.nodes.get(self.nodes.len() - 1)
    }

    pub fn set_child_parent(&mut self, child_index: usize, parent_index: usize) {
        match self.nodes.get_mut(child_index) {
            Some(child) => {
                child.parent = Some(parent_index);
            },
            None => {}
        }
    }

    pub fn next_index(&self) -> usize {
        self.nodes.len()
    }

    pub fn add_node(&mut self, node: Node) {
        self.nodes.push(node);
    }
    
    pub fn parent(&self, node: &Node) -> Option<&Node> {
        let parent_index = node.parent?;
        self.nodes.get(parent_index)
    }

    pub fn children<'a, 'b>(&'a self, node: &'b Node) -> HashMap<Path, &'a Node> {
        node.nodes.iter()
        .filter_map(|(path,(index, _selector))| {
            self.nodes.get(*index).and_then(|node| {Some((path.clone(), node))})
        }).collect()
    }
}

#[derive(Clone)]
pub struct Node {
    path: Path,
    rules: Vec<Box<dyn Rule>>,
    nodes: HashMap<Path, (usize, Option<Box<dyn PropertyMapper>>)>,
    parent: Option<usize>

}

impl Node {
    pub fn new(
        path: Path,
        rules: Vec<Box<dyn Rule>>,
    ) -> Self {
        Self {
            path,
            rules,
            nodes: HashMap::default(),
            parent: None
        }
    }
    pub fn set_nodes(&mut self, nodes: HashMap<Path, (usize, Option<Box<dyn PropertyMapper>>)>) {
        self.nodes = nodes;
    }
    pub fn set_parent(&mut self, parent: usize) {
        self.parent = Some(parent);
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rules(&self) -> Vec<Box<dyn Rule>> {
        self.rules.clone()
    }
}

pub struct RuleResult(pub String, pub bool, pub String);

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

impl <R> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    NoFold,
    NoInit,
    NoAssert,
>
where
    R: Clone + 'static
{
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

impl <R, Acc> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    NoInit,
    NoAssert,
>
where
    Acc: Send + Clone + 'static,
    R: Clone + 'static
{
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

impl <R, Acc> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    NoAssert,
>
where
    Acc: Send + Clone + 'static,
    R: Clone + 'static
{
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

impl <R, Acc> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R + Send + Sync>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
>
where
    Acc: Sync + Send + Clone + 'static,
    R: Clone + 'static
{
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

#[derive(Clone)]
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

impl <Acc, R> Rule for ConcreteRule<Acc, R>
    where
        Acc: Sync + Send + Clone + 'static,
        R: Clone + 'static
{

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

    pub fn new(attrs: HashMap<String, String>) -> Self {
        Self {
            attrs,
            text: None

        }
    }
    pub fn set_text(&mut self, text: &str) {
        self.text = Some(String::from(text))
    }
    pub fn text(&self) -> Option<&String> {
        self.text.as_ref()
    }

    pub fn attr(&self, key: &str) -> Option<&String> {
        self.attrs.get(key)
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub struct Path(pub String);

impl ToString for Path {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}