use std::{cell::RefCell, collections::HashMap, fmt::Display, pin::Pin, rc::Rc, sync::Arc};
use tokio::sync::{Mutex, broadcast::Receiver};
use std::fmt::Debug;
use educe::Educe;

#[derive(Debug)]
pub struct BuilderError(String);

impl Display for BuilderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error: {}", self.0)
    }
}

pub trait NodeHandler {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: usize);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Box<dyn Rule>) -> Self::AddRuleOutput;
    fn path(self, path: Path, map_view: bool, map_children: Vec<Vec<Path>>) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

pub trait Rule
    where Self: Sync + Send + DynCloneRule + Debug
{
    fn fold(&mut self, view: Arc<Mutex<FullNodeView>>, ctx: Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Pin<Box<dyn Future<Output = ()> + Send + Sync + '_>>;
    fn assert(&self, path: &str) -> Diagnostic;
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
    map_view: bool,
    map_children: Vec<Vec<Path>>,
    nodes: HashMap<Path, usize>,
    rules: Vec<Box<dyn Rule>>
}

impl NodeHandler for Root<NodeAdderState> {
    type ToBuilderOuput = Root<InitState>;
    fn add_node(&mut self, path: Path, node: usize) {
        self.nodes.insert(path, node);
    }
    fn to_builder(self) -> Self::ToBuilderOuput {
        Root {
            _state: std::marker::PhantomData,
            tree: self.tree,
            nodes: self.nodes,
            path: self.path,
            map_view: self.map_view,
            map_children: self.map_children,
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

    fn path(self, path: Path, map_view: bool, map_children: Vec<Vec<Path>>) -> Self::PathOutput {
        let build_parent: Root<NodeAdderState> = Root {
            _state: std::marker::PhantomData,
            tree: Rc::clone(&self.tree),
            nodes: self.nodes,
            path: self.path,
            map_view: self.map_view,
            map_children: self.map_children,
            rules: self.rules
        };
        Child::new(build_parent, Rc::clone(&self.tree), path, map_view, map_children)
    }

    fn build(self) -> Self::BuildOutput {

        // we create the parent node
        let mut parent = Node::new(
            self.path,
            self.rules,
            self.map_view,
            self.map_children
        );
        match Rc::try_unwrap(self.tree) {
            Ok(tree) => {
                let mut tree = tree.into_inner();
                // for every child it declare, we must set it's parent with the help of the tree
                let parent_index = tree.next_index();
                for (_1, node) in &self.nodes {
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
    pub fn new(root: &str, map_view: bool, map_children: Vec<Vec<Path>>) -> Self {
        Self {
            _state: std::marker::PhantomData,
            tree: Rc::new(RefCell::new(Tree::new())),
            path: Path(root.into()),
            nodes: HashMap::new(),
            map_view,
            map_children,
            rules: Vec::new()
        }
    }
}

pub struct Child<State, Parent> {
    _state: std::marker::PhantomData<State>,
    tree: Rc<RefCell<Tree>>,
    map_view: bool,
    map_children: Vec<Vec<Path>>,
    parent: Parent,
    path: Path,
    nodes: HashMap<Path, usize>,
    rules: Vec<Box<dyn Rule>>
}

impl <P: NodeHandler> NodeHandler for Child<NodeAdderState, P> {
    
    type ToBuilderOuput = Child<InitState, P>;

    fn add_node(&mut self, path: Path, node: usize) {
        self.nodes.insert(path, node);
    }
    
    fn to_builder(self) -> Self::ToBuilderOuput {
        Child {
            _state: std::marker::PhantomData,
            tree: self.tree,
            nodes: self.nodes,
            map_view: self.map_view,
            map_children: self.map_children,
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

    fn path(self, path: Path, map_view: bool, map_children: Vec<Vec<Path>>) -> Self::PathOutput
        where Child<NodeAdderState, P>: NodeHandler
    {
        let build_parent = Child {
            _state: std::marker::PhantomData,
            tree: Rc::clone(&self.tree),
            map_view: self.map_view,
            map_children: self.map_children,
            parent: self.parent,
            nodes: self.nodes,
            path: self.path,
            rules: self.rules
        };
        Child::new(build_parent, Rc::clone(&self.tree), path, map_view, map_children)
    }

    fn build(mut self) -> Self::BuildOutput {
        let mut parent = Node::new(
            self.path.clone(),
            self.rules,
            self.map_view,
            self.map_children
        );
        let mut tree = self.tree.borrow_mut();
        // for every child it declare, we must set it's parent with the help of the tree
        let parent_index = tree.next_index();
        for (_1, node) in &self.nodes {
            tree.set_child_parent(*node, parent_index);
        }
        parent.set_nodes(self.nodes);
        // we add the node to the vector
        tree.add_node(parent);

        self.parent.add_node(self.path, parent_index);

        self.parent.to_builder()
    }
}

impl <P: NodeHandler> Child<InitState, P> {
    pub fn new(
        parent: P,
        tree: Rc<RefCell<Tree>>,
        path: Path,
        map_view: bool,
        map_children: Vec<Vec<Path>>
    ) -> Child<InitState, P> {
        Child {
            _state: std::marker::PhantomData,
            tree,
            path,
            map_view,
            map_children,
            parent,
            nodes: HashMap::new(),
            rules: Vec::new()
        }
    }
}

#[derive(Clone, Debug)]
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

    pub fn children<'b>(&self, node: &'b Node) -> HashMap<Path, &Node> {
        node.nodes.iter()
        .filter_map(|(path,index)| {
            self.nodes.get(*index).and_then(|node| {Some((path.clone(), node))})
        }).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    path: Path,
    rules: Vec<Box<dyn Rule>>,
    map_view: bool,
    map_children: Vec<Vec<Path>>,
    nodes: HashMap<Path, usize>,
    parent: Option<usize>

}

impl Node {
    pub fn new(
        path: Path,
        rules: Vec<Box<dyn Rule>>,
        map_view: bool,
        map_children: Vec<Vec<Path>>
    ) -> Self {
        Self {
            path,
            rules,
            map_view,
            map_children,
            nodes: HashMap::default(),
            parent: None
        }
    }
    pub fn set_nodes(&mut self, nodes: HashMap<Path, usize>) {
        self.nodes = nodes;
    }
    pub fn nodes(&self) -> &HashMap<Path, usize> {
        &self.nodes
    }
    pub fn set_parent(&mut self, parent: usize) {
        self.parent = Some(parent);
    }

    pub fn map_view(&self) -> bool {
        self.map_view
    }

    pub fn map_children(&self) -> &Vec<Vec<Path>> {
        &self.map_children
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn rules(&self) -> Vec<Box<dyn Rule>> {
        self.rules.clone()
    }
}

#[derive(Debug)]
pub struct RuleResult(pub String, pub String, pub bool, pub String);

pub struct NoTest;
pub struct NoFold;
pub struct NoInit;
pub struct NoAssert;

type AsyncTestFn<R> = Arc<dyn Fn(Arc<Mutex<FullNodeView>>, Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Pin<Box<dyn Future<Output = R> + Send + Sync>> + Send + Sync>;

pub trait AsyncTest<R> {
    fn call(& self, view: Arc<Mutex<FullNodeView>>, ctx: Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Pin<Box<dyn Future<Output = R> + Send + Sync + 'static>>;
}

impl <'a, R, F, Fut> AsyncTest<R> for F
where
    F: Fn(Arc<Mutex<FullNodeView>>, Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Fut + Send + Sync,
    Fut: Future<Output = R> + Send + Sync + 'static
{
    fn call(&self, view: Arc<Mutex<FullNodeView>>, ctx: Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Pin<Box<dyn Future<Output = R> + Send + Sync + 'static>> {
        Box::pin(self(view, ctx))
    }
}

pub struct RuleBuilder<
    TestType,
    FoldType,
    InitType,
    AssertType,
> {
    name: String,
    test: TestType,
    fold: FoldType,
    init: InitType,
    assert: AssertType,
}
impl <'a> RuleBuilder<
    NoTest,
    NoFold,
    NoInit,
    NoAssert,
> {
    pub fn test<R>(name: String, test: Arc<dyn AsyncTest<R> + Sync + Send>) -> RuleBuilder<
        Arc<dyn AsyncTest<R> + Sync + Send>,
        NoFold,
        NoInit,
        NoAssert,
    > { 
        RuleBuilder {
            name,
            test: test,
            fold: NoFold,
            init: NoInit,
            assert: NoAssert,
        }
    }
}

impl <R> RuleBuilder<
    Arc<dyn AsyncTest<R> + Sync + Send>,
    NoFold,
    NoInit,
    NoAssert,
>
where
    R: Clone + 'static
{
    pub fn fold<Acc: Send + 'static>(self, fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>) -> RuleBuilder<
        Arc<dyn AsyncTest<R> + Sync + Send>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        NoInit,
        NoAssert,
    > {
        RuleBuilder {
            name: self.name,
            test: self.test,
            fold: fold,
            init: NoInit,
            assert: NoAssert,
        }
    }
}

impl <R, Acc> RuleBuilder<
    Arc<dyn AsyncTest<R> + Sync + Send>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    NoInit,
    NoAssert,
>
where
    Acc: Send + Clone + 'static,
    R: Clone + 'static
{
    pub fn init(self, init: Box<dyn Fn() -> Acc>) -> RuleBuilder<
        Arc<dyn AsyncTest<R> + Sync + Send>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        Box<dyn Fn() -> Acc>,
        NoAssert,
    > {
        RuleBuilder {
            name: self.name,
            test: self.test,
            fold: self.fold,
            init: init,
            assert: NoAssert,
        }
    }
}

impl <R, Acc> RuleBuilder<
    Arc<dyn AsyncTest<R> + Sync + Send>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    NoAssert,
>
where
    Acc: Send + Clone + 'static,
    R: Clone + 'static
{
    pub fn assert(self, assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>) -> RuleBuilder<
        Arc<dyn AsyncTest<R> + Sync + Send>,
        Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        Box<dyn Fn() -> Acc>,
        Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
    > {
        RuleBuilder {
            name: self.name,
            test: self.test,
            fold: self.fold,
            init: self.init,
            assert: assert,
        }
    }
}

impl <R, Acc> RuleBuilder<
    Arc<dyn AsyncTest<R> + Sync + Send>,
    Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    Box<dyn Fn() -> Acc>,
    Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
>
where
    Acc: Debug + Sync + Send + Clone + 'static,
    R: Clone + 'static
{
    pub fn build(&self, assertion: &'static str) -> Box<dyn Rule> {
        Box::new(
            ConcreteRule::new(
                self.name.clone(),
                (self.init)(),
                Arc::clone(&self.test),
                Arc::clone(&self.fold),
                Arc::clone(&self.assert),
                assertion
            )
        )
    }
}

#[derive(Clone, Educe)]
#[educe(Debug)]
pub struct ConcreteRule<Acc: Send + Debug + 'static, R: 'static> {
    name: String,
    state: Acc,
    #[educe(Debug(ignore))]
    test: Arc<dyn AsyncTest<R> + Sync + Send>,
    #[educe(Debug(ignore))]
    fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
    #[educe(Debug(ignore))]
    assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
    assertion: &'static str
}

impl <Acc: Debug + Send + 'static, R: 'static> ConcreteRule<Acc, R> {
    pub fn new(
        name: String,
        init: Acc,
        test: Arc<dyn AsyncTest<R> + Sync + Send>,
        fold: Arc<dyn Fn(&Acc, R) -> Acc + Send + Sync>,
        assert: Arc<dyn Fn(&Acc) -> bool + Send + Sync>,
        assertion: &'static str, 
    ) -> Self {
        Self {
            name,
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
        Acc: Debug + Sync + Send + Clone + 'static,
        R: Clone + 'static
{
    fn fold(&mut self, view: Arc<Mutex<FullNodeView>>, ctx: Arc<HashMap<Vec<Path>, Mutex<PartialNodeView>>>) -> Pin<Box<dyn Future<Output = ()> + Send + Sync + '_>> {
        Box::pin(async move {
            self.state = (self.fold)(&self.state, self.test.call(view, ctx).await);
        })
    }

    fn assert(&self, path: &str) -> Diagnostic {
        Diagnostic {
            rule_name: self.name.clone(),
            assertion: substitute(self.assertion, path, &self.name),
            statut: (self.assert)(&self.state)
        }
    }
}

pub struct Diagnostic {
    pub rule_name: String,
    pub assertion: String,
    pub statut: bool
}

pub trait CommonNodeView {
    fn text(&self) -> Arc<Receiver<String>>;
    fn attr(&self, key: &str) -> Option<&String>;
    fn index(&self) -> usize;
}

#[derive(Debug)]
pub struct FullNodeView {
    index: usize,
    text: Arc<Receiver<String>>,
    attrs: HashMap<String, String>,
    children: Arc<HashMap<Vec<Path>, Receiver<PartialNodeView>>>
}

impl FullNodeView {
    pub fn new(
        attrs: HashMap<String, String>,
        index: usize,
        receiver: Arc<Receiver<String>>,
        children: Arc<HashMap<Vec<Path>, Receiver<PartialNodeView>>>
    ) -> Self {
        Self {
            index,
            attrs,
            text: receiver,
            children
        }
    }

    pub fn children(&self) -> Arc<HashMap<Vec<Path>, Receiver<PartialNodeView>>> {
        self.children.clone()
    }
}

impl CommonNodeView for FullNodeView {
    fn text(&self) -> Arc<Receiver<String>> {
        self.text.clone()
    }

    fn attr(&self, key: &str) -> Option<&String> {
        self.attrs.get(key)
    }

    fn index(&self) -> usize {
        self.index
    }
}

#[derive(Debug, Clone)]
pub struct PartialNodeView {
    index: usize,
    text: Arc<Receiver<String>>,
    attrs: HashMap<String, String>
}

impl PartialNodeView {

    pub fn new(attrs: HashMap<String, String>, index: usize, receiver: Arc<Receiver<String>>) -> Self {
        Self {
            index,
            attrs,
            text: receiver
        }
    }
}

impl <'a> CommonNodeView for PartialNodeView {
    fn text(&self) -> Arc<Receiver<String>> {
        self.text.clone()
    }

    fn attr(&self, key: &str) -> Option<&String> {
        self.attrs.get(key)
    }

    fn index(&self) -> usize {
        self.index
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub struct Path(pub String);

impl ToString for Path {
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

fn substitute(template: &str, path: &str, rule: &str) -> String {
    template
        .replace("{path}", path)
        .replace("{rule}", rule)
}