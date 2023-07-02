use async_trait::async_trait;
use erased_serde::Deserializer;
use futures::Future;
use serde::{de::DeserializeOwned, ser::SerializeMap, Deserialize, Serialize, Serializer};
use std::{
    any::Any, collections::HashMap, convert::Infallible, error::Error, marker::PhantomData,
    pin::Pin
};

use super::ToolUseError;

/// Represents a single parameter for a tool.
#[derive(Clone, Debug)]
pub struct FormatPart {
    pub key: String,
    pub purpose: String,
}

impl FormatPart {
    /// Creates a new `FormatPart` with the given key and purpose.
    pub fn new(key: &str, purpose: &str) -> Self {
        FormatPart {
            key: key.to_string(),
            purpose: purpose.to_string(),
        }
    }
}

impl<K: Into<String>, P: Into<String>> From<(K, P)> for FormatPart {
    fn from((k, p): (K, P)) -> Self {
        FormatPart::new(&k.into(), &p.into())
    }
}

/// Represents the format for a tool's input or output.
#[derive(Debug)]
pub struct Format {
    pub parts: Vec<FormatPart>,
}

impl Format {
    /// Creates a new `Format` with the given parts.
    pub fn new(parts: Vec<FormatPart>) -> Self {
        Format { parts }
    }
}

impl<T: AsRef<[FormatPart]>> From<T> for Format {
    fn from(parts: T) -> Self {
        Format::new(parts.as_ref().to_vec())
    }
}

impl Serialize for Format {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let n = self.parts.len();
        let mut map = serializer.serialize_map(Some(n))?;
        for part in &self.parts {
            map.serialize_entry(&part.key, &part.purpose)?;
        }
        map.end()
    }
}

/// A trait to provide a description format for a tool.
pub trait Describe {
    fn describe() -> Format;
}


/// Represents the description of a tool, including its name, usage, and input/output formats.
#[derive(Serialize, Debug)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub description_context: String,
    pub input_format: Format,
    // #[serde(skip)]
    // #[allow(dead_code)]
    /// This will be used in the future.
    pub output_format: Format,
}

impl ToolDescription {
    /// Creates a new `ToolDescription` with the given name, description, context, and formats.
    pub fn new(
        name: &str,
        description: &str,
        description_context: &str,
        input_format: Format,
        output_format: Format,
    ) -> Self {
        ToolDescription {
            name: name.to_string(),
            description: description.to_string(),
            description_context: description_context.to_string(),
            input_format,
            output_format,
        }
    }
}
pub struct Yaml<T: DeserializeOwned + Send>(pub T);

pub struct State<T>(pub T);

impl<S> FromContext<S> for State<S> {
    type Error = Infallible;
    fn from_context(_message: &str, state: S) -> Result<Self, Self::Error> {
        Ok(State(state))
    }
}

impl<T> Describe for Yaml<T>
where
    T: Describe + DeserializeOwned + Send,
{
    fn describe() -> Format {
        T::describe()
    }
}

// TODO: This blanket impl does not provide LLMs with info about the Error and its possible values;
//       To have this capability we should figure out a new version of Format
//          that would properly convey that enums can have either keys from variant 1
//          or variant 2 but not both
//       For now Tools return Results so errors are passed to the Toolbox and up into some top-level
//          code, so that users can handle those. LLMs are not aware of these errors because they are not part of Format description.
//       If you want your LLM to be aware of Tool errors you can work around this limitation
//          by including an Option<> field in your Tool's output and describing it as containing
//          either the Error value or nothing. 
impl<T, E> Describe for Result<T, E>
where 
    T: Describe 
{
    fn describe() -> Format {
        T::describe()
    }
}

impl<T: DeserializeOwned + Send + ToString> ToString for Yaml<T> {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

pub trait FromContext<S>: Sized {
    type Error: ToString;
    fn from_context(message: &str, state: S) -> Result<Self, Self::Error>;
}

impl<S, T: DeserializeOwned + Send> FromContext<S> for Yaml<T> {
    type Error = serde_yaml::Error;
    fn from_context(message: &str, _state: S) -> Result<Self, Self::Error> {
        Ok(Yaml(serde_yaml::from_str(&message)?))
    }
}

pub trait Handler<T, S>: Send + Sync + Sized + 'static {
    type Output: ToString;
    type Error: Error;
    type Future: Future<Output = Result<Self::Output, Box<Self::Error>>> + Send;
    fn call(self, message: String, state: S) -> Self::Future;
    fn with_state(self, state: S) -> HandlerService<Self, T, S, Self::Error>;
}


impl<F, S, Fut, Res, Err> Handler<(), S> for F
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send,
    Res: ToString + Describe,
    Err: Error,
{
    type Output = String;
    type Error = Err;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Box<Self::Error>>> + Send>>;
    fn call(self, _context: String, _state: S) -> Self::Future {
        Box::pin(async move { (self)().await.map(|v| v.to_string()).map_err(Box::new) })
    }

    fn with_state(self, state: S) -> HandlerService<Self, (), S, Self::Error> {
        HandlerService::new(self, state, Format { parts: vec![] }, Res::describe())
    }
}


macro_rules! impl_from_context {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[allow(non_snake_case)]
        impl<S, $($ty,)* $last> FromContext<S> for ($($ty,)* $last,)
        where
            $( $ty: FromContext<S> + Send, )*
            $last: FromContext<S> + Send,
            S: Clone + Send + Sync,
        {
            type Error = String;
            fn from_context(req: &str, state: S) -> Result<Self, Self::Error> {
                $(
                    let $ty = $ty::from_context(req, state.clone()).map_err(|e| e.to_string())?;
                )*

                let $last = $last::from_context(req, state).map_err(|e| e.to_string())?;

                Ok(($($ty,)* $last,))
            }
        }
    };
}

macro_rules! impl_handler {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[allow(non_snake_case, unused_mut)]
        impl<F, S, Fut, Res, Err, $($ty,)* $last> Handler<($($ty,)* $last,), S> for F
        where
            F: FnOnce($($ty,)* $last,) -> Fut + Send + Sync + 'static,
            Fut: Future<Output = Result<Res, Err>> + Send,
            Res: ToString + Describe,
            Err: Error,
            S: Clone + Send + Sync + 'static,
            $( $ty: FromContext<S> + Send, )*
            $last: FromContext<S> + Send + Describe,
        {
            type Output = String;
            type Error = Err;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Box<Self::Error>>> + Send>>;

            fn call(self, req: String, state: S) -> Self::Future {
                Box::pin(async move {
                $(
                    let $ty = match $ty::from_context(&req, state.clone()) {
                        Ok(val) => val,
                        Err(err) => return Ok(err.to_string()),
                    };
                )*

                let $last = match $last::from_context(&req, state) {
                    Ok(val) => val,
                    Err(err) => return Ok(err.to_string()),
                };

                match self($($ty,)* $last,).await {
                    Ok(val) => Ok(val.to_string()),
                    Err(err) => Err(Box::new(err))
                }
            })
            }

            fn with_state(self, state: S) -> HandlerService<Self, ($($ty,)* $last,), S, Err> {
                HandlerService::new(self, state, $last::describe(), Res::describe())
            }
        }
    };
}

macro_rules! impl_pipe {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        impl<$($ty,)* F1, O1, Fut1> Pipe<($($ty,)*), O1> for F1
        where
            F1: FnOnce($($ty,)*) -> Fut1 + Sized,
            Fut1: Future<Output = O1>,
        {
            fn pipe<O2: ToString + Describe, Err2: Error, Fut2: Future<Output = Result<O2, Err2>>, F2: FnOnce(O1) -> Fut2>(
                self,
                f: F2,
            ) -> PipedFn<Self, F2> {
                PipedFn { fn1: self, fn2: f }
            }
        }
    }
}

macro_rules! impl_pipe_handler {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[allow(non_snake_case)]
        impl<$($ty,)* $last, S, F1, F2, Fut1, Res1, Fut2, Res2, Err2> Handler<($($ty,)* $last,), S> for PipedFn<F1, F2>
        where
            F1: FnOnce($($ty,)* $last) -> Fut1 + Send + Sync + 'static,
            F2: FnOnce(Res1) -> Fut2 + Send + Sync + 'static,
            $( $ty: FromContext<S> + Send, )*
            $last: FromContext<S> + Send + Describe,
            Fut1: Future<Output = Res1> + Send,
            Fut2: Future<Output = Result<Res2, Err2>> + Send,
            Res1: Send,
            Res2: ToString + Describe,
            Err2: Error,
            S: Clone + Send + 'static,
        {
            type Output = String;
            type Error = Err2;
            type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Box<Self::Error>>> + Send>>;

            fn call(self, message: String, state: S) -> Self::Future {
                Box::pin(async move {
                    $(
                        let $ty = match $ty::from_context(&message, state.clone()) {
                            Ok(val) => val,
                            Err(err) => return Ok(err.to_string()),
                        };
                    )*
                    let $last = match $last::from_context(&message, state) {
                        Ok(val) => val,
                        Err(err) => return Ok(err.to_string()),
                    };
                    let res1 = (self.fn1)($($ty,)* $last).await;
                    (self.fn2)(res1).await.map(|v| v.to_string()).map_err(|e| Box::new(e))
                })
            }

            fn with_state(self, state: S) -> HandlerService<Self, ($($ty,)* $last,), S, Self::Error> {
                HandlerService::new(self, state, $last::describe(), Res2::describe())
            }
        }
    };
}
#[rustfmt::skip]
macro_rules! all_the_tuples {
    ($name:ident) => {
        $name!([], T1);
        $name!([T1], T2);
        $name!([T1, T2], T3);
        $name!([T1, T2, T3], T4);
        $name!([T1, T2, T3, T4], T5);
        $name!([T1, T2, T3, T4, T5], T6);
        $name!([T1, T2, T3, T4, T5, T6], T7);
        $name!([T1, T2, T3, T4, T5, T6, T7], T8);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8], T9);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9], T10);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10], T11);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11], T12);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12], T13);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13], T14);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14], T15);
        $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15], T16);
    };
}

all_the_tuples!(impl_from_context);
all_the_tuples!(impl_handler);
all_the_tuples!(impl_pipe);
all_the_tuples!(impl_pipe_handler);

/// TOOL OUTPUT HANDLING
pub trait Pipe<T, O1>: Sized {
    fn pipe<O2: ToString + Describe, Err2: Error, Fut2: Future<Output = Result<O2, Err2>>, F: FnOnce(O1) -> Fut2>(
        self,
        f: F,
    ) -> PipedFn<Self, F>;
}

#[derive(Clone)]
pub struct PipedFn<F1, F2> {
    fn1: F1,
    fn2: F2,
}

impl<T, S, F1, F2, Fut1, Res1, Fut2, Res2> Handler<T, S> for PipedFn<F1, F2>
where
    F1: FnOnce(T) -> Fut1 + Send + Sync + 'static,
    F2: FnOnce(Res1) -> Fut2 + Send + Sync + 'static,
    T: FromContext<S> + Send + Describe,
    Fut1: Future<Output = Res1> + Send,
    Fut2: Future<Output = Res2> + Send,
    Res1: Send,
    Res2: ToString + Describe,
    S: Send + 'static,
{
    type Output = String;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Output, Box<Self::Error>>> + Send>>;

    fn call(self, message: String, state: S) -> Self::Future {
        Box::pin(async move {
            let t = match T::from_context(&message, state) {
                Ok(val) => val,
                Err(err) => return Ok(err.to_string()),
            };
            let res1 = (self.fn1)(t).await;
            Ok((self.fn2)(res1).await.to_string())
        })
    }

    fn with_state(self, state: S) -> HandlerService<Self, T, S, Self::Error> {
        HandlerService::new(self, state, T::describe(), Res2::describe())
    }
}

/// ROUTING
pub struct HandlerService<H, T, S, E> {
    handler: H,
    state: S,
    name: String,
    usage_description: String,
    input_description: Format,
    output_description: Format,
    _marker: PhantomData<fn() -> Result<T, E>>,
}

impl<H, T, S, E> HandlerService<H, T, S, E> {
    pub fn new(
        handler: H,
        state: S,
        input_description: Format,
        output_description: Format,
    ) -> Self {
        Self {
            handler,
            state,
            input_description,
            output_description,
            _marker: Default::default(),
            name: "".into(),
            usage_description: "".into(),
        }
    }
}

#[async_trait]
pub trait Tool<E: Error> {
    async fn call(&self, message: String) -> Result<String, Box<E>>;
}

#[async_trait]
impl<H, T, S, Err> Tool<Err> for HandlerService<H, T, S, Err>
where
    Err: Error,
    H: Handler<T, S, Error = Err> + Clone,
    S: Clone + Send + Sync,
{
    async fn call(&self, message: String) -> Result<String, Box<Err>> {
        let handler = self.handler.clone();
        match handler.call(message, self.state.clone()).await {
            Ok(val) => Ok(val.to_string()),
            Err(e) => Ok(e.to_string()),
        }
    }
}

pub struct Toolbox<'a, E: Error = ToolUseError<'a>> {
    tools: HashMap<String, Box<dyn Tool<E>>>,
    _marker: PhantomData<&'a E>,
}

impl<'a, E: Error + Clone> Toolbox<'a, E> {
    fn add_tool<T, E2>(&mut self, tool: T)
    where
        T: Tool<E> + 'static,
    {
        self.tools.insert("abc".into(), Box::new(tool));
    }

    async fn invoke(&self, name: &str, message: &str) -> Option<Result<String, E>> {
        if let Some(tool) = self.tools.get(name) {
            match tool.call(message.into()).await {
                Ok(_) => todo!(),
                Err(_) => todo!(),
            }
        } else {
            None
        }
        
    }
}