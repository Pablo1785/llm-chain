use std::{collections::HashMap, marker::PhantomData, any::Any, pin::Pin};
use erased_serde::Deserializer;
use async_trait::async_trait;
use futures::Future;
use serde::{ser::SerializeMap, Serialize, Serializer, de::DeserializeOwned};

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

pub struct NotFoundError;

pub trait Tool<S> {
    type Future: Future<Output = serde_yaml::Value>;
    fn call(&self, input: serde_yaml::Value, state: S) -> Self::Future;
    fn describe_input(&self) -> Format;
    fn describe_output(&self) -> Format;
}

pub struct DescribedTool<T, S> where T: Tool<S> {
    tool: Box<T>,
    name: String,
    description: String,
    _marker: PhantomData<S>
}

impl<T: Tool<S>, S> DescribedTool<T, S> {
    pub async fn call(&self, name: &str, input: &serde_yaml::Value) -> serde_yaml::Value {
        todo!()
    }

    pub fn description(&self) -> ToolDescription {
        todo!()
    }
}

type BoxedFuture = Box<dyn Future<Output = serde_yaml::Value>>;
type BoxedTool<S> = Box<dyn Tool<S, Future = BoxedFuture>>;

pub struct Toolbox<S> {
    tools: HashMap<String, BoxedTool<S>>,
    state: S,
}

impl<S> Toolbox<S> {
    pub fn add_tool<T: Tool<S, Future = BoxedFuture>>(&mut self, name: &str, description: &str, tool: T) -> Option<BoxedTool<S>> {
        self.tools.insert(name.into(), Box::new(tool))
    }

    pub fn describe_all_tools(&self) -> Result<serde_yaml::Value, serde_yaml::Error> {
        todo!()
    }

    pub async fn call_tool(&self, name: &str, input: &serde_yaml::Value) -> Result<serde_yaml::Value, NotFoundError> {
        todo!()
    }
}

impl<F, Fut, Res, S> Tool<S> for F
where
    F: FnOnce() -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Res> + Send,
    Res: Serialize + Describe,
    S: Clone
{
    type Future = Pin<Box<dyn Future<Output = serde_yaml::Value> + Send>>;

    fn call(&self, _req: serde_yaml::Value, _state: S) -> Self::Future {
        Box::pin(async move { serde_yaml::to_value(self().await) })
    }
}



// macro_rules! impl_from_request {
//     (
//         [$($ty:ident),*], $last:ident
//     ) => {

//         // This impl must not be generic over M, otherwise it would conflict with the blanket
//         // implementation of `FromRequest<S, Mut>` for `T: FromRequestParts<S>`.
//         #[async_trait]
//         #[allow(non_snake_case, unused_mut, unused_variables)]
//         impl<S, $($ty,)* $last> FromRequest<S> for ($($ty,)* $last,)
//         where
//             $( $ty: FromRequest<S> + Send, )*
//             $last: FromRequest<S> + Send,
//             S: Send + Sync,
//         {
//             type Rejection = Response;

//             async fn from_request(req: &serde_yaml::Value, state: &S) -> Result<Self, Self::Rejection> {
//                 $(
//                     let $ty = $ty::from_request(req, state).await.map_err(|rejection| rejection.into_response())?;
//                 )*

//                 let $last = $last::from_request(&req, state).await.map_err(|rejection| rejection.into_response())?;

//                 Ok(($($ty,)* $last,))
//             }
//         }
//     };
// }
// macro_rules! impl_handler {
//     (
//         [$($ty:ident),*], $last:ident
//     ) => {
//         #[allow(non_snake_case, unused_mut)]
//         impl<F, Fut, S, M, $($ty,)* $last> Handler<(M, $($ty,)* $last,), S> for F
//         where
//             F: FnOnce($($ty,)* $last,) -> Fut + Clone + Send,
//             Fut: Future<Output = Response> + Send,
//             S: Clone + Send + Sync + 'static,
//             $( $ty: FromRequest<S, M> + Send, )*
//             $last: FromRequest<S, M> + Send,
//         {
//             type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

//             fn call(self, req: serde_yaml::Value, state: S) -> Self::Future {
//                 Box::pin(async move {
//                     let state = &state;

//                     $(
//                         let $ty = match $ty::from_request(&req, state).await {
//                             Ok(value) => value,
//                             Err(rejection) => return rejection.into_response(),
//                         };
//                     )*

//                     let $last = match $last::from_request(&req, state).await {
//                         Ok(value) => value,
//                         Err(rejection) => return rejection.into_response(),
//                     };

//                     let res = self($($ty,)* $last,).await;

//                     res.into_response()
//                 })
//             }
//         }
//     };
// }

// #[rustfmt::skip]
// macro_rules! all_the_tuples {
//     ($name:ident) => {
//         $name!([], T1);
//         $name!([T1], T2);
//         $name!([T1, T2], T3);
//         $name!([T1, T2, T3], T4);
//         $name!([T1, T2, T3, T4], T5);
//         $name!([T1, T2, T3, T4, T5], T6);
//         $name!([T1, T2, T3, T4, T5, T6], T7);
//         $name!([T1, T2, T3, T4, T5, T6, T7], T8);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8], T9);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9], T10);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10], T11);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11], T12);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12], T13);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13], T14);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14], T15);
//         $name!([T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14, T15], T16);
//     };
// }

// all_the_tuples!(impl_from_request);
// all_the_tuples!(impl_handler);