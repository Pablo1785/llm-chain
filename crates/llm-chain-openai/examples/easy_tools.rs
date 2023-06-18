use std::{
    collections::HashMap, convert::Infallible, future::Future, marker::PhantomData, pin::Pin, sync::{Arc},
};

use async_trait::async_trait;
use llm_chain::{
    document_stores::in_memory_document_store::InMemoryDocumentStore, traits::VectorStore, schema::{EmptyMetadata, Document}, tools::{Format, Describe, FormatPart, Yaml, State, Tool, Handler, Pipe},
};

use llm_chain_hnsw::{HnswVectorStore, HnswArgs};
use llm_chain_macros::Describe;
use llm_chain_openai::embeddings::Embeddings;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::Mutex;


/// ======================================== USER CODE
#[derive(Serialize, Deserialize, Describe)]
struct MyStruct {
    #[purpose("Unique struct identifier")]
    id: u32,
    #[purpose("Name of the struct")]
    name: String,
}

impl ToString for MyStruct {
    fn to_string(&self) -> String {
        match serde_yaml::to_string(self) {
            Ok(val) => val,
            Err(err) => err.to_string(),
        }
    }
}


async fn print_yaml(
    State(MyComplicatedState { num, .. }): State<MyComplicatedState>,
    Yaml(MyStruct { id, name }): Yaml<MyStruct>,
) -> Yaml<MyStruct> {
    let text: String = MyStruct::describe()
        .parts
        .iter()
        .map(|FormatPart { key, purpose }| format!("key: {key} purpose: {purpose}"))
        .collect();
    println!("FORMAT: {text}");
    println!("State: {num}");
    Yaml(MyStruct { id, name })
}

/// SIMPLE EXAMPLE OF A TOOL
struct MyTool {
    id: u32,
    name: String,
    data: Vec<u8>,
}

#[async_trait]
impl Tool for MyTool {
    async fn call(&self, message: String) -> String {
        format!(
            "Hello from my tool, LLM! You've asked me to {}. I've got {}, {}, and {:?}",
            message, self.id, self.name, self.data
        )
    }
}

/// TOOL THAT HAS TO BE PIPED TO HANDLE ERRORS
#[derive(Debug)]
struct MyError(pub String);

async fn failable_tool(
    State(MyComplicatedState { num, .. }): State<MyComplicatedState>,
    Yaml(MyStruct { id, name }): Yaml<MyStruct>,
) -> Result<String, MyError> {
    println!("State: {num}");
    if id > 18 {
        Ok(name)
    } else {
        Err(MyError("ID was not above 18".into()))
    }
}

async fn my_error_handler(res: Result<String, MyError>) -> MyStruct {
    match res {
        Ok(val) => MyStruct { id: 19, name: val },
        Err(MyError(err_msg)) => MyStruct {
            id: 0,
            name: format!("Bad luck: {err_msg}"),
        },
    }
}

async fn tool_that_uses_different_state(State(num): State<f64>, Yaml(input): Yaml<MyStruct>) -> MyStruct {
    println!("Different state: {num}");
    input
}

/// USING VECTORSTORES WITH THE NEW TOOLS
async fn vectorstore_tool(
    State(MyComplicatedState {
        hnsw_vector_store, ..
    }): State<MyComplicatedState>,
    Yaml(MySimilaritySearchInput { query }): Yaml<MySimilaritySearchInput>,
) -> MySimilaritySearchOutput {
    match hnsw_vector_store.similarity_search(query, 1).await {
        Ok(docs) if docs.len() == 1 => MySimilaritySearchOutput {
            most_similar_text: docs[0].page_content.clone(),
            optional_error: None,
        },
        Ok(docs) if docs.len() > 1 => MySimilaritySearchOutput { most_similar_text: "".into(), optional_error: Some(MyError("Query executed correctly but more than one document was returned".into())) },
        Ok(docs) => MySimilaritySearchOutput { most_similar_text: "".into(), optional_error: Some(MyError("Query executed correctly but no documents were found".into())) },
        Err(err) => MySimilaritySearchOutput {
            most_similar_text: "".into(),
            optional_error: Some(MyError(err.to_string())),
        },
    }
}

#[derive(Clone)]
struct MyComplicatedState {
    num: u32,
    hnsw_vector_store: Arc<HnswVectorStore<Embeddings, InMemoryDocumentStore<()>, ()>>,
}

#[derive(Describe, Deserialize)]
struct MySimilaritySearchInput {
    #[purpose("Text which you search for in the vectorstore")]
    query: String,
    // Notice that the model no longer has to specify the limit - we can hardcode it in the function
}

/// ERROR DESCRIPTION WILL GET NICER AS SOON AS WE HAVE A DERIVE Describe FOR ENUMS;
/// THERE WILL BE A BLANKET IMPL FOR Result<> AND Option<>
#[derive(Describe)]
struct MySimilaritySearchOutput {
    #[purpose("Text that is most similar to the one you searched for")]
    most_similar_text: String,
    #[purpose("This will be empty if there was no error")]
    optional_error: Option<MyError>,
}

impl ToString for MySimilaritySearchOutput {
    fn to_string(&self) -> String {
        if let Some(ref err_msg) = self.optional_error {
            format!("There was an error with the vectorstore: {err_msg:#?}")
        } else {
            self.most_similar_text.clone()
        }
    }
}

fn example_documents() -> Vec<Document<()>> {
    let doc_dog_definition = r#"The dog (Canis familiaris[4][5] or Canis lupus familiaris[5]) is a domesticated descendant of the wolf. Also called the domestic dog, it is derived from the extinct Pleistocene wolf,[6][7] and the modern wolf is the dog's nearest living relative.[8] Dogs were the first species to be domesticated[9][8] by hunter-gatherers over 15,000 years ago[7] before the development of agriculture.[1] Due to their long association with humans, dogs have expanded to a large number of domestic individuals[10] and gained the ability to thrive on a starch-rich diet that would be inadequate for other canids.[11]

    The dog has been selectively bred over millennia for various behaviors, sensory capabilities, and physical attributes.[12] Dog breeds vary widely in shape, size, and color. They perform many roles for humans, such as hunting, herding, pulling loads, protection, assisting police and the military, companionship, therapy, and aiding disabled people. Over the millennia, dogs became uniquely adapted to human behavior, and the human–canine bond has been a topic of frequent study.[13] This influence on human society has given them the sobriquet of "man's best friend"."#.to_string();

    let doc_woodstock_sound = r#"Sound for the concert was engineered by sound engineer Bill Hanley. "It worked very well", he says of the event. "I built special speaker columns on the hills and had 16 loudspeaker arrays in a square platform going up to the hill on 70-foot [21 m] towers. We set it up for 150,000 to 200,000 people. Of course, 500,000 showed up."[48] ALTEC designed marine plywood cabinets that weighed half a ton apiece and stood 6 feet (1.8 m) tall, almost 4 feet (1.2 m) deep, and 3 feet (0.91 m) wide. Each of these enclosures carried four 15-inch (380 mm) JBL D140 loudspeakers. The tweeters consisted of 4×2-Cell & 2×10-Cell Altec Horns. Behind the stage were three transformers providing 2,000 amperes of current to power the amplification setup.[49][page needed] For many years this system was collectively referred to as the Woodstock Bins.[50] The live performances were captured on two 8-track Scully recorders in a tractor trailer back stage by Edwin Kramer and Lee Osbourne on 1-inch Scotch recording tape at 15 ips, then mixed at the Record Plant studio in New York.[51]"#.to_string();

    let doc_reddit_creep_shots = r#"A year after the closure of r/jailbait, another subreddit called r/CreepShots drew controversy in the press for hosting sexualized images of women without their knowledge.[34] In the wake of this media attention, u/violentacrez was added to r/CreepShots as a moderator;[35] reports emerged that Gawker reporter Adrian Chen was planning an exposé that would reveal the real-life identity of this user, who moderated dozens of controversial subreddits, as well as a few hundred general-interest communities. Several major subreddits banned links to Gawker in response to the impending exposé, and the account u/violentacrez was deleted.[36][37][38] Moderators defended their decisions to block the site from these sections of Reddit on the basis that the impending report was "doxing" (a term for exposing the identity of a pseudonymous person), and that such exposure threatened the site's structural integrity.[38]"#.to_string();

    
            vec![
                doc_dog_definition,
                doc_woodstock_sound,
                doc_reddit_creep_shots,
            ]
            .into_iter()
            .map(Document::new)
            .collect()
       
}

#[tokio::main]
pub async fn main() {
    let message = "
            id: 23
            name: Abc
    "
    .to_string();

    println!("OPENAI KEY: {}", std::env::var("OPENAI_API_KEY").unwrap());
    let embeddings = llm_chain_openai::embeddings::Embeddings::default();
    let document_store = Arc::new(Mutex::new(InMemoryDocumentStore::<()>::new()));
    let hnsw_vs = Arc::new(HnswVectorStore::new(HnswArgs::default(), Arc::new(embeddings), document_store));
    hnsw_vs
    .add_documents(
        example_documents(),
    )
    .await
    .unwrap();
    let my_complicated_state = MyComplicatedState {
        num: 221,
        hnsw_vector_store: hnsw_vs,
    };

    let mut handlers: HashMap<String, Box<dyn Tool>> = HashMap::new();

    handlers.insert("yaml".to_string(), Box::new(print_yaml.with_state(my_complicated_state.clone())));
    handlers.insert(
        "pipe".to_string(),
        Box::new(failable_tool.pipe(my_error_handler).with_state(my_complicated_state.clone())),
    );
    handlers.insert(
        "similarity".to_string(),
        Box::new(vectorstore_tool.with_state(my_complicated_state.clone())),
    );
    handlers.insert("Other state".to_string(), Box::new(tool_that_uses_different_state.with_state(12.5)));

    let res = handlers.get("pipe").unwrap().call(message.clone()).await;
    println!("Pipe response: {res}");

    let res = handlers.get("yaml").unwrap().call(message).await;
    println!("Yaml response: {res}");

    let res = handlers.get("similarity").unwrap().call("
            query: Some controversial topic
    ".to_string()).await;
    println!("Similarity response: {res}");
}
