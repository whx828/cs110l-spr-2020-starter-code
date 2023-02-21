use std::fmt;
use std::option::Option;

pub struct LinkedList<T> {
    head: Option<Box<Node<T>>>,
    size: usize,
}

struct Node<T> {
    value: T,
    next: Option<Box<Node<T>>>,
}

impl<T> Node<T> {
    pub fn new(value: T, next: Option<Box<Node<T>>>) -> Node<T> {
        Node {
            value: value,
            next: next,
        }
    }
}

impl<T> Clone for Node<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Node {
            value: self.value.clone(),
            next: self.next.clone(),
        }
    }
}

impl<T> PartialEq for Node<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value && self.next == other.next
    }
}

impl<T> LinkedList<T> {
    pub fn new() -> LinkedList<T> {
        LinkedList {
            head: None,
            size: 0,
        }
    }

    pub fn get_size(&self) -> usize {
        self.size
    }

    pub fn is_empty(&self) -> bool {
        self.get_size() == 0
    }

    pub fn push_front(&mut self, value: T) {
        let new_node: Box<Node<T>> = Box::new(Node::new(value, self.head.take()));
        self.head = Some(new_node);
        self.size += 1;
    }

    pub fn pop_front(&mut self) -> Option<T> {
        let node: Box<Node<T>> = self.head.take()?;
        self.head = node.next;
        self.size -= 1;
        Some(node.value)
    }
}

impl<T> Clone for LinkedList<T>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        LinkedList {
            head: self.head.clone(),
            size: self.size,
        }
    }
}

impl<T> PartialEq for LinkedList<T>
where
    T: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        self.head == other.head && self.size == other.size
    }
}

impl<T> fmt::Display for LinkedList<T>
where
    T: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut current: &Option<Box<Node<T>>> = &self.head;
        let mut result = String::new();
        loop {
            match current {
                Some(node) => {
                    result = format!("{} {}", result, node.value);
                    current = &node.next;
                }
                None => break,
            }
        }
        write!(f, "{}", result)
    }
}

impl<T> Drop for LinkedList<T> {
    fn drop(&mut self) {
        let mut current = self.head.take();
        while let Some(mut node) = current {
            current = node.next.take();
        }
    }
}

pub struct LinkedListIter<'a, T> {
    current: &'a Option<Box<Node<T>>>,
}

impl<T> Iterator for LinkedListIter<'_, T>
where
    T: Clone,
{
    type Item = T;
    fn next(&mut self) -> Option<Self::Item> {
        match self.current {
            Some(node) => {
                let item = node.value.clone();
                self.current = &node.next;
                Some(item)
            }
            None => None,
        }
    }
}

impl<'a, T> IntoIterator for &'a LinkedList<T>
where
    T: Clone,
{
    type Item = T;
    type IntoIter = LinkedListIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        LinkedListIter {
            current: &self.head,
        }
    }
}

pub trait ComputeNorm {
    fn compute_norm(&self) -> f64 {
        0.0
    }
}

impl ComputeNorm for LinkedList<f64> {
    fn compute_norm(&self) -> f64 {
        self.into_iter().map(|x| x * x).sum::<f64>().sqrt()
    }
}
