//! Minimal DOM and [`TreeSink`] implementation for the HTML parser.
//!
//! Vendored from `markup5ever_rcdom` 0.39.0+unofficial (MIT/Apache-2.0, same as
//! this crate) because upstream ships that helper with `publish = false` as an
//! unsupported test-only DOM, so no official release tracks `html5ever` 0.40.
//! Depending on the unofficial re-publish pinned `tuika-html` to
//! `markup5ever` 0.39; owning this copy removes that pin.
//!
//! Adaptations from the original:
//!
//! - The serializer (`SerializableHandle` and everything below it) is dropped;
//!   `tuika-html` walks the tree directly and never serializes.
//! - The `<selectedcontent>` cloning override
//!   (`maybe_clone_an_option_into_selectedcontent` plus the `Node` helpers only
//!   it used) is dropped. The [`TreeSink`] trait provides a no-op default, so
//!   omitting the override only means an enabled `<selectedcontent>` element
//!   keeps no cloned copy of the selected `<option>` — invisible to this
//!   renderer, which has no `<select>`/`<option>` handling at all. Dropping it
//!   also removes the only use of `xml5ever`, so no new dependency is needed.
//! - Imports come through `html5ever`'s re-exports instead of `markup5ever`
//!   directly, so this module moves in lockstep with the single HTML parser
//!   dependency.

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use html5ever::interface::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, ExpandedName, QualName};

/// A node handle: a strong reference-counted pointer to a [`Node`].
pub(crate) type Handle = Rc<Node>;

/// A weak node handle, used for parent pointers so the tree has no cycles.
type WeakHandle = Weak<Node>;

/// One node of the parsed document tree.
pub(crate) struct Node {
    /// Strong references to this node's children, in document order.
    pub(crate) children: RefCell<Vec<Handle>>,
    /// This node's payload. Never changes after creation.
    pub(crate) data: NodeData,
    /// This node's parent, if it has been attached to the tree yet.
    parent: Cell<Option<WeakHandle>>,
}

/// The payload of a [`Node`].
///
/// The parser produces every variant, but the renderer only reads `Document`,
/// `Text`, and `Element` — the rest exist so the tree is complete.
#[derive(Debug)]
#[allow(dead_code)]
pub(crate) enum NodeData {
    /// The root of the document tree.
    Document,

    /// A `<!DOCTYPE …>` preamble.
    Doctype {
        /// The doctype name (usually `html`).
        name: StrTendril,
        /// The doctype public identifier, if any.
        public_id: StrTendril,
        /// The doctype system identifier, if any.
        system_id: StrTendril,
    },

    /// A run of character data.
    Text {
        /// The text. Accumulated in place while tokenizing.
        contents: RefCell<StrTendril>,
    },

    /// An `<!-- … -->` comment.
    Comment {
        /// The comment text.
        contents: StrTendril,
    },

    /// An element with its tag name and attributes.
    Element {
        /// The element's tag name.
        name: QualName,
        /// The element's attributes, in source order.
        attrs: RefCell<Vec<Attribute>>,
        /// For `<template>`: the template contents fragment. For anything
        /// else: always `None`.
        template_contents: RefCell<Option<Handle>>,
        /// Whether this is a MathML annotation-xml integration point.
        mathml_annotation_xml_integration_point: bool,
    },

    /// A `<? … ?>` processing instruction.
    ProcessingInstruction {
        /// The instruction target.
        target: StrTendril,
        /// The instruction data.
        contents: StrTendril,
    },
}

impl Node {
    /// Create a detached node holding `data`.
    fn new(data: NodeData) -> Handle {
        Handle::new(Node {
            children: Default::default(),
            data,
            parent: Default::default(),
        })
    }

    /// This node's parent and this node's index within it, if attached.
    fn get_parent_and_index(target: &Handle) -> Option<(Handle, usize)> {
        if let Some(weak) = target.parent.take() {
            let parent = weak.upgrade();
            debug_assert!(parent.is_some());
            target.parent.set(Some(weak));
            parent.as_ref().and_then(|parent| {
                parent
                    .children
                    .borrow()
                    .iter()
                    .position(|child| Rc::ptr_eq(child, target))
                    .map(|index| (parent.clone(), index))
            })
        } else {
            None
        }
    }

    /// Attach `child` to `parent`, appending it to the child list.
    fn append(parent: &Handle, child: Handle) {
        child.parent.set(Some(Rc::downgrade(parent)));
        parent.children.borrow_mut().push(child);
    }

    /// Detach `target` from its parent, if it has one.
    fn remove_from_parent(target: &Handle) {
        if let Some((parent, i)) = Node::get_parent_and_index(target) {
            parent.children.borrow_mut().remove(i);
        }
    }

    /// Append `text` to a text node, merging adjacent character runs.
    fn append_to_existing_text(prev: &Handle, text: &str) -> bool {
        let prev = prev.clone();
        if let NodeData::Text { contents } = &prev.data {
            contents.borrow_mut().push_slice(text);
            true
        } else {
            false
        }
    }
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node")
            .field("children", &self.children)
            .field("data", &self.data)
            .finish()
    }
}

/// The parsed document: its root plus the parser's out-of-band state.
pub(crate) struct RcDom {
    /// The `Document` node everything hangs off.
    pub(crate) document: Handle,
    /// Parse errors reported through [`TreeSink::parse_error`].
    errors: RefCell<Vec<Cow<'static, str>>>,
    /// The quirks mode from the doctype.
    quirks_mode: Cell<QuirksMode>,
}

impl TreeSink for RcDom {
    type Output = Handle;
    type Handle = Handle;
    type ElemName<'a> = ExpandedName<'a>;

    fn finish(self) -> Handle {
        self.document
    }

    fn parse_error(&self, msg: Cow<'static, str>) {
        self.errors.borrow_mut().push(msg);
    }

    fn get_document(&self) -> Handle {
        self.document.clone()
    }

    fn get_template_contents(&self, target: &Handle) -> Handle {
        template_contents_of(target)
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        Rc::ptr_eq(x, y)
    }

    fn elem_name<'a>(&self, target: &'a Handle) -> ExpandedName<'a> {
        let NodeData::Element { name, .. } = &target.data else {
            panic!("not an element!")
        };
        name.expanded()
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        Node::new(NodeData::Element {
            name,
            attrs: RefCell::new(attrs),
            template_contents: RefCell::new(if flags.template {
                Some(Node::new(NodeData::Document))
            } else {
                None
            }),
            mathml_annotation_xml_integration_point: flags.mathml_annotation_xml_integration_point,
        })
    }

    fn create_comment(&self, text: StrTendril) -> Handle {
        Node::new(NodeData::Comment { contents: text })
    }

    fn create_pi(&self, target: StrTendril, data: StrTendril) -> Handle {
        Node::new(NodeData::ProcessingInstruction {
            target,
            contents: data,
        })
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        match child {
            NodeOrText::AppendNode(node) => Node::append(parent, node),
            NodeOrText::AppendText(text) => {
                let last_child = parent.children.borrow().last().cloned();
                if !last_child.is_some_and(|child| Node::append_to_existing_text(&child, &text)) {
                    let node = Node::new(NodeData::Text {
                        contents: RefCell::new(text),
                    });
                    Node::append(parent, node);
                }
            }
        }
    }

    fn append_before_sibling(&self, sibling: &Handle, child: NodeOrText<Handle>) {
        let (parent, i) = Node::get_parent_and_index(sibling)
            .expect("append_before_sibling called on node without parent");

        let child = match (child, i) {
            // No previous node.
            (NodeOrText::AppendText(text), 0) => Node::new(NodeData::Text {
                contents: RefCell::new(text),
            }),

            // Look for a text node before the insertion point.
            (NodeOrText::AppendText(text), i) => {
                let children = parent.children.borrow();
                let prev = &children[i - 1];
                if Node::append_to_existing_text(prev, &text) {
                    return;
                }
                Node::new(NodeData::Text {
                    contents: RefCell::new(text),
                })
            }

            // The tree builder promises we won't have a text node after
            // the insertion point.

            // Any other kind of node.
            (NodeOrText::AppendNode(node), _) => node,
        };

        Node::remove_from_parent(&child);

        child.parent.set(Some(Rc::downgrade(&parent)));
        parent.children.borrow_mut().insert(i, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        if Node::get_parent_and_index(element).is_some() {
            self.append_before_sibling(element, child)
        } else {
            self.append(prev_element, child)
        }
    }

    fn append_doctype_to_document(
        &self,
        name: StrTendril,
        public_id: StrTendril,
        system_id: StrTendril,
    ) {
        Node::append(
            &self.document,
            Node::new(NodeData::Doctype {
                name,
                public_id,
                system_id,
            }),
        );
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let NodeData::Element {
            attrs: element_attrs,
            ..
        } = &target.data
        else {
            panic!("not an element!")
        };

        let mut existing = element_attrs
            .borrow()
            .iter()
            .map(|attribute| (attribute.name.clone(), ()))
            .collect::<HashSet<_>>();
        let mut element_attrs = element_attrs.borrow_mut();
        for attr in attrs {
            if existing.insert((attr.name.clone(), ())) {
                element_attrs.push(attr);
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        if let Some((parent, index)) = Node::get_parent_and_index(target) {
            parent.children.borrow_mut().remove(index);
            target.parent.set(None);
        }
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let children = node.children.take();
        for child in &children {
            child.parent.set(Some(Rc::downgrade(new_parent)));
        }
        new_parent.children.borrow_mut().extend(children);
    }

    fn set_quirks_mode(&self, mode: QuirksMode) {
        self.quirks_mode.set(mode);
    }

    fn associate_with_form(
        &self,
        _target: &Handle,
        _form_element: &Handle,
        _nodes: (&Handle, Option<&Handle>),
    ) {
    }

    fn is_mathml_annotation_xml_integration_point(&self, target: &Handle) -> bool {
        if let NodeData::Element {
            mathml_annotation_xml_integration_point,
            ..
        } = target.data
        {
            mathml_annotation_xml_integration_point
        } else {
            panic!("not an element!")
        }
    }
}

impl Default for RcDom {
    fn default() -> RcDom {
        RcDom {
            document: Node::new(NodeData::Document),
            errors: Default::default(),
            quirks_mode: Cell::new(QuirksMode::NoQuirks),
        }
    }
}

/// The template-contents fragment of a `<template>` element handle.
///
/// Split out so tests can reach the fragment without a sink instance.
pub(crate) fn template_contents_of(template: &Handle) -> Handle {
    if let NodeData::Element {
        template_contents, ..
    } = &template.data
    {
        template_contents
            .borrow()
            .as_ref()
            .expect("template: no contents")
            .clone()
    } else {
        panic!("not a template element!")
    }
}
