use std::{collections::HashMap, marker::PhantomData, any::Any, pin::Pin};
use erased_serde::Deserializer;
use async_trait::async_trait;
use futures::Future;
use serde::{ser::SerializeMap, Serialize, Serializer, de::DeserializeOwned, Deserialize};

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

#[derive(Clone, Serialize)]
pub struct NotFoundError;

pub trait ConstDescribe {
    const FORMAT: Format;
}

impl ConstDescribe for NotFoundError {
    const FORMAT: Format = Format { parts: todo!() }; 
}

impl<T> Describe for T where T: ConstDescribe {
    fn describe() -> Format {
        Self::FORMAT
    }
}

type YamlResult = Result<serde_yaml::Value, serde_yaml::Error>;

#[async_trait]
pub trait Tool<T: ?Sized, S> {
    async fn call(&self, input: &serde_yaml::Value, state: &S) -> YamlResult;
    fn describe_input(&self) -> Format;
    fn describe_output(&self) -> Format;
}

pub struct DescribedTool<T: ?Sized, S> {
    tool: Box<dyn Tool<T, S>>,
    name: String,
    description: String
}

pub struct Toolbox<S> {
    tools: HashMap<String, DescribedTool<dyn Any, S>>,
    state: S,
}

impl<S> Toolbox<S> {
    pub fn add_tool(&mut self, name: &str, description: &str, tool: impl Tool<dyn Any, S> + 'static) -> Option<DescribedTool<dyn Any, S>> {
        self.tools.insert(name.into(), DescribedTool { tool: Box::new(tool), name: name.into(), description: description.into() })
    }

    pub fn describe_all_tools(&self) -> Result<serde_yaml::Value, serde_yaml::Error> {
        todo!()
    }

    pub async fn call_tool(&self, name: &str, input: &serde_yaml::Value) -> Result<serde_yaml::Value, NotFoundError> {
        todo!()
    }
}

pub trait FromState<S> {
    fn from_state(state: S) -> Self;
}


#[async_trait]
impl<F, Fut, Res, S> Tool<(), S> for F
where
    F: FnOnce() -> Fut + Clone + Send + Sync,
    Fut: Future<Output = Res> + Send,
    Res: Serialize + ConstDescribe,
    S: Clone + Send
{
    async fn call(&self, _req: &serde_yaml::Value, _state: &S) -> YamlResult {
        serde_yaml::to_value(self.clone()().await)
    }
    fn describe_input(&self) -> Format {
        Format { parts: vec![] }
    }

    fn describe_output(&self) -> Format {
        Res::FORMAT
    }
}



macro_rules! impl_from_state {
    (
        [$($ty:ident),*], $last:ident
    ) => {

        // This impl must not be generic over M, otherwise it would conflict with the blanket
        // implementation of `FromRequest<S, Mut>` for `T: FromRequestParts<S>`.
        impl<S, $($ty,)* $last> FromState<S> for ($($ty,)* $last,)
        where
            $( $ty: FromState<S> + Send, )*
            $last: FromState<S> + Send,
            S: Clone + Send + Sync,
        {
            fn from_state(state: S) -> Self {
                $(
                    let $ty = $ty::from_state(state.clone());
                )*

                let $last = $last::from_state(state.clone());

                ($($ty,)* $last,)
            }
        }
    };
}
macro_rules! impl_handler {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[async_trait]
        impl<F, Fut, S, Res, $($ty,)* $last> Tool<($($ty,)* $last,), S> for F
        where
            F: FnOnce( $($ty,)* $last, ) -> Fut + Clone + Send + Sync,
            Fut: Future<Output = Res> + Send,
            Res: Serialize + ConstDescribe,
            S: Clone + Send + Sync,
            $( $ty: FromState<S> + Send + Sync, )*
            $last: DeserializeOwned + Send + ConstDescribe,
        {

            async fn call(&self, input: &serde_yaml::Value, state: &S) -> YamlResult {
                    $(
                        let $ty = $ty::from_state(state.clone());
                    )*

                    let $last = serde_yaml::from_value(input.clone())?;

                    let res = self.clone()($($ty,)* $last,).await;

                    serde_yaml::to_value(res)
            }
            fn describe_input(&self) -> Format {
                $last::FORMAT
            }

            fn describe_output(&self) -> Format {
                Res::FORMAT
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

all_the_tuples!(impl_from_state);
all_the_tuples!(impl_handler);


async fn my_tool() -> NotFoundError {
    NotFoundError
}

#[derive(Deserialize)]
struct ToolInput { 
    pub text: String
}

impl ConstDescribe for ToolInput {
    const FORMAT: Format = Format { parts: vec![FormatPart { key: "text".to_string(), purpose: "Text of the input".to_string() }]};
}

async fn my_input_tool(ToolInput {text }: ToolInput) -> NotFoundError {
    NotFoundError
}

fn do_smth() {
    let tool = Box::new(my_tool) as Box<dyn Tool<(), usize>>;
    tool.describe_input();

    let tool2 = Box::new(my_input_tool) as Box<dyn Tool<(ToolInput,), usize>>;
}