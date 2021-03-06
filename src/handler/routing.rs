//!Routing related traits and types.

use std::collections::HashMap;
use std::iter::{Iterator, FlatMap};
use std::slice::Split;
use std::ops::Deref;

use context::MaybeUtf8Owned;

///A segmented route.
pub trait Route<'a> {
    ///An iterator over route segments.
    type Segments: Iterator<Item=&'a [u8]>;

    ///Create a route segment iterator. The iterator is expected to return
    ///None for a root path (`/`).
    ///
    ///```rust
    ///# use rustful::handler::routing::Route;
    ///let root = "/";
    ///assert_eq!(root.segments().next(), None);
    ///
    ///let path = ["/path", "to/somewhere/", "/", "/else/"];
    ///let segments = path.segments().collect::<Vec<_>>();
    ///let expected = vec![
    ///    "path".as_bytes(),
    ///    "to".as_bytes(),
    ///    "somewhere".as_bytes(),
    ///    "else".as_bytes()
    ///];
    ///assert_eq!(segments, expected);
    ///```
    fn segments(&'a self) -> <Self as Route<'a>>::Segments;
}

fn is_slash(c: &u8) -> bool {
    *c == b'/'
}

const IS_SLASH: &'static fn(&u8) -> bool = & (is_slash as fn(&u8) -> bool);

impl<'a> Route<'a> for str {
    type Segments = RouteIter<Split<'a, u8, &'static fn(&u8) -> bool>>;

    fn segments(&'a self) -> <Self as Route<'a>>::Segments {
        self.as_bytes().segments()
    }
}

impl<'a> Route<'a> for [u8] {
    type Segments = RouteIter<Split<'a, u8, &'static fn(&u8) -> bool>>;

    fn segments(&'a self) -> <Self as Route<'a>>::Segments {
        let s = if self.starts_with(b"/") {
            &self[1..]
        } else {
            self
        };
        let s = if s.ends_with(b"/") {
            &s[..s.len() - 1]
        } else {
            s
        };

        if s.len() == 0 {
            RouteIter::Root
        } else {
            RouteIter::Path(s.split(IS_SLASH))
        }
    }
}


impl<'a, 'b: 'a, I: 'a, T: 'a> Route<'a> for I where
    &'a I: IntoIterator<Item=&'a T>,
    T: Deref,
    <T as Deref>::Target: Route<'a> + 'b
{
    type Segments = FlatMap<<&'a I as IntoIterator>::IntoIter, <<T as Deref>::Target as Route<'a>>::Segments, fn(&'a T) -> <<T as Deref>::Target as Route<'a>>::Segments>;

    fn segments(&'a self) -> Self::Segments {
        fn segments<'a, 'b: 'a, T: Deref<Target=R> + 'b, R: ?Sized + Route<'a, Segments=S> + 'b, S: Iterator<Item=&'a[u8]>>(s: &'a T) -> S {
            s.segments()
        }

        self.into_iter().flat_map(segments)
    }
}

///Utility iterator for when a root path may be hard to represent.
#[derive(Clone)]
pub enum RouteIter<I: Iterator> {
    ///A root path (`/`).
    Root,
    ///A non-root path (`path/to/somewhere`).
    Path(I)
}

impl<I: Iterator> Iterator for RouteIter<I> {
    type Item = <I as Iterator>::Item;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        match *self {
            RouteIter::Path(ref mut i) => i.next(),
            RouteIter::Root => None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match *self {
            RouteIter::Path(ref i) => i.size_hint(),
            RouteIter::Root => (0, Some(0))
        }
    }
}

///A state object for routing.
#[derive(Clone)]
pub struct RouteState<'a> {
    route: Vec<&'a [u8]>,
    variables: Vec<Option<usize>>,
    index: usize,
    var_index: usize,
}

impl<'a> RouteState<'a> {
    ///Get the current path segment.
    pub fn get(&self) -> Option<&'a [u8]> {
        self.route.get(self.index).cloned()
    }

    ///Don't include this path segment in a variable.
    pub fn skip(&mut self) {
        self.variables.get_mut(self.index).map(|v| *v = None);
        self.index += 1;
    }

    ///Include this path segment as a variable.
    pub fn keep(&mut self) {
        let v_i = self.var_index;
        self.variables.get_mut(self.index).map(|v| *v = Some(v_i));
        self.index += 1;
        self.var_index += 1;
    }

    ///Extend a previously saved variable value with this path segment, or
    ///save it as a new variable.
    pub fn fuse(&mut self) {
        let v_i = self.var_index;
        self.variables.get_mut(self.index).map(|v| *v = Some(v_i));
        self.index += 1;
    }

    ///Assign names to the saved variables and return them.
    pub fn variables(&self, names: &[MaybeUtf8Owned]) -> HashMap<MaybeUtf8Owned, MaybeUtf8Owned> {
        let values = self.route.iter().zip(self.variables.iter()).filter_map(|(v, keep)| {
            if let Some(index) = *keep {
                Some((index, *v))
            } else {
                None
            }
        });

        let mut var_map = HashMap::<MaybeUtf8Owned, MaybeUtf8Owned>::with_capacity(names.len());
        for (name, value) in VariableIter::new(names, values) {
            var_map.insert(name, value);
        }

        var_map
    }

    ///Get a snapshot of a part of the current state.
    pub fn snapshot(&self) -> (usize, usize) {
        (self.index, self.var_index)
    }

    ///Go to a previously recorded state.
    pub fn go_to(&mut self, snapshot: (usize, usize)) {
        let (index, var_index) = snapshot;
        self.index = index;
        self.var_index = var_index;
    }

    ///Check if there are no more segments.
    pub fn is_empty(&self) -> bool {
        self.index == self.route.len()
    }
}

impl<'a, R: Route<'a> + ?Sized> From<&'a R> for RouteState<'a> {
    fn from(route: &'a R) -> RouteState<'a> {
        let route: Vec<_> = route.segments().collect();
        RouteState {
            variables: vec![None; route.len()],
            route: route,
            index: 0,
            var_index: 0,
        }
    }
}

struct VariableIter<'a, I> {
    iter: I,
    names: &'a [MaybeUtf8Owned],
    current: Option<(usize, MaybeUtf8Owned, MaybeUtf8Owned)>
}

impl<'a, I: Iterator<Item=(usize, &'a [u8])>> VariableIter<'a, I> {
    fn new(names: &'a [MaybeUtf8Owned], iter: I) -> VariableIter<'a, I> {
        VariableIter {
            iter: iter,
            names: names,
            current: None
        }
    }
}

impl<'a, I: Iterator<Item=(usize, &'a [u8])>> Iterator for VariableIter<'a, I> {
    type Item=(MaybeUtf8Owned, MaybeUtf8Owned);

    fn next(&mut self) -> Option<Self::Item> {
        for (next_index, next_segment) in &mut self.iter {
            //validate next_index and check if the variable has a name
            debug_assert!(next_index < self.names.len(), format!("invalid variable name index! variable_names.len(): {}, index: {}", self.names.len(), next_index));
            let next_name = match self.names.get(next_index) {
                None => continue,
                Some(n) if n.is_empty() => continue,
                Some(n) => n
            };

            if let Some((index, name, mut segment)) = self.current.take() {
                if index == next_index {
                    //this is a part of the current sequence
                    segment.push_char('/');
                    segment.push_bytes(next_segment);
                    self.current = Some((index, name, segment));
                } else {
                    //the current sequence has ended
                    self.current = Some((next_index, (*next_name).clone(), next_segment.to_owned().into()));
                    return Some((name, segment));
                }
            } else {
                //this is the first named variable
                self.current = Some((next_index, (*next_name).clone(), next_segment.to_owned().into()));
            }
        }

        //return the last variable
        self.current.take().map(|(_, name, segment)| (name, segment))
    }
}
