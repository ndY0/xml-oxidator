use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use futures::{Stream, StreamExt};

pub trait NodeHandler<'a> {
    type ToBuilderOuput;
    fn add_node(&mut self, path: Path, node: Node<'a>, map_from_parent: Option<&'a ParentPropertyMapper<'a>>);
    fn to_builder(self) -> Self::ToBuilderOuput;
}

pub trait NodeBuilder<'a> {
    type AddRuleOutput;
    type PathOutput;
    type BuildOutput;
    fn add_rule(self, rule: Rule) -> Self::AddRuleOutput;
    fn path(self, path: Path, map_from_parent: Option<&'a ParentPropertyMapper<'a>>) -> Self::PathOutput;
    fn build(self) -> Self::BuildOutput;
}

pub struct InitState;
pub struct NodeAdderState;

pub struct Root<'a, State> {
    _state: std::marker::PhantomData<State>,
    path: Path,
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Rule>
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
        Node::new(
            self.rules,
            self.nodes
        )
    }
}

impl <'a> Root<'a, InitState> {
    pub fn new() -> Self {
        Self {
            _state: std::marker::PhantomData,
            path: Path::Root,
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
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,
    rules: Vec<Rule>
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
            rules: self.rules
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
        let current_node = Node::new(
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
    rules: Vec<Rule>,
    nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>,

}

impl <'a> Node<'a> {
    pub fn new(
        rules: Vec<Rule>,
        nodes: HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)>
    ) -> Self {
        Self {
            rules,
            nodes
        }
    }
    
    pub fn rules(&self) -> &Vec<Rule> {
        &self.rules
    }

    pub fn children(&self) -> &HashMap<Path, (Node<'a>, Option<&'a ParentPropertyMapper<'a>>)> {
        &self.nodes
    }

    pub async fn run(&self, stream: Pin<Box<dyn Stream<Item = NodeView>>>, ctx: &HashMap<String, String>) -> Vec<RuleResult> {

            while let Some(view) = stream.next().await {
                for rule in self.rules {
                    
                }
                acc = fold(acc, test(&view, ctx))
            }
            assert(&acc)
        }
        let test = Arc::clone(&self.test);
        let fold = Arc::clone(&self.fold);
        let assert = Arc::clone(&self.assert);
        let init = Arc::clone(&self.init);
        Rule {
            assertion: assertion.into(),
            test_fold_assert: Box::new(
                move |mut stream, ctx| {
                        let mut acc = (init)();
                        let test = Arc::clone(&test);
                        let fold = Arc::clone(&fold);
                        let assert = Arc::clone(&assert);
                    Box::pin(async move {
                        while let Some(view) = stream.next().await {
                            acc = fold(acc, test(&view, ctx))
                        }
                        assert(&acc)
                    })
                }
            )
        }
    }
}

pub struct RuleResult(Path, bool, String);

pub struct NoTest;
pub struct NoFold;
pub struct NoInit;
pub struct NoAssert;
pub struct NoAssertion;

pub struct RuleBuilder<
    TestType,
    FoldType,
    InitType,
    AssertType,
    AssertionType
> {
    test: TestType,
    fold: FoldType,
    init: InitType,
    assert: AssertType,
    assertion: NoAssertion
}
impl RuleBuilder<
    NoTest,
    NoFold,
    NoInit,
    NoAssert,
    NoAssertion
> {
    pub fn test<R>(test: Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
        NoFold,
        NoInit,
        NoAssert,
        NoAssertion
    > {
        RuleBuilder {
            test: test,
            fold: NoFold,
            init: NoInit,
            assert: NoAssert,
            assertion: NoAssertion
        }
    }
}

impl <R: 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
    NoFold,
    NoInit,
    NoAssert,
    NoAssertion
> {
    pub fn fold<Acc>(self, fold: Arc<dyn Fn(Acc, R) -> Acc>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
        Arc<dyn Fn(Acc, R) -> Acc>,
        NoInit,
        NoAssert,
        NoAssertion
    > {
        RuleBuilder {
            test: self.test,
            fold: fold,
            init: NoInit,
            assert: NoAssert,
            assertion: NoAssertion
        }
    }
}

impl <R: 'static, Acc: 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
    Arc<dyn Fn(Acc, R) -> Acc>,
    NoInit,
    NoAssert,
    NoAssertion
> {
    pub fn init(self, init: Arc<dyn Fn() -> Acc>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
        Arc<dyn Fn(Acc, R) -> Acc>,
        Arc<dyn Fn() -> Acc>,
        NoAssert,
        NoAssertion
    > {
        RuleBuilder {
            test: self.test,
            fold: self.fold,
            init: init,
            assert: NoAssert,
            assertion: NoAssertion
        }
    }
}

impl <R: 'static, Acc: 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
    Arc<dyn Fn(Acc, R) -> Acc>,
    Arc<dyn Fn() -> Acc>,
    NoAssert
> {
    pub fn assert(self, assert: Arc<dyn Fn(&Acc) -> bool>) -> RuleBuilder<
        Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
        Arc<dyn Fn(Acc, R) -> Acc>,
        Arc<dyn Fn() -> Acc>,
        Arc<dyn Fn(&Acc) -> bool>
    > {
        RuleBuilder {
            test: self.test,
            fold: self.fold,
            init: self.init,
            assert: assert,
            assertion: NoAssertion
        }
    }
}

impl <R: 'static, Acc: 'static> RuleBuilder<
    Arc<dyn Fn(&NodeView, &HashMap<String, String>) -> R>,
    Arc<dyn Fn(Acc, R) -> Acc>,
    Arc<dyn Fn() -> Acc>,
    Arc<dyn Fn(&Acc) -> bool>,
    String
> {
    pub fn build(&self, assertion: &str) -> Rule {
        let test = Arc::clone(&self.test);
        let fold = Arc::clone(&self.fold);
        let assert = Arc::clone(&self.assert);
        let init = Arc::clone(&self.init);
        Rule {
            assertion: assertion.into(),
            test_fold_assert: Box::new(
                move |mut stream, ctx| {
                        let mut acc = (init)();
                        let test = Arc::clone(&test);
                        let fold = Arc::clone(&fold);
                        let assert = Arc::clone(&assert);
                    Box::pin(async move {
                        while let Some(view) = stream.next().await {
                            acc = fold(acc, test(&view, ctx))
                        }
                        assert(&acc)
                    })
                }
            )
        }
    }
}



pub struct Rule {
    test_fold_assert: Box<dyn for<'a> Fn(Pin<Box<dyn Stream<Item = NodeView> + 'a>>, &'a HashMap<String, String>) -> Pin<Box<dyn Future<Output = bool> + 'a>>>,
    assertion: String
}

impl Rule {
    pub fn test<E>(&self, view: NodeView) -> E {
        self.test(view)
    }
    pub fn apply<'a>(&self, stream: Pin<Box<dyn Stream<Item = NodeView>>>, ctx: &'a HashMap<String, String>) ->Pin<Box<dyn futures::Future<Output = bool> + 'a>> {
        (self.test_fold_assert)(stream, ctx)
    }
    pub fn assertion(&self) -> &String {
        &self.assertion
    }
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