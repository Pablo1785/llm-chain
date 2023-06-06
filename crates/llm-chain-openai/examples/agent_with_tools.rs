
use llm_chain::agents::conversational::Agent;
use llm_chain::agents::self_ask_with_search::EarlyStoppingConfig;
use llm_chain::executor;
use llm_chain::multitool;
use llm_chain::parameters;

use llm_chain::prompt::{ConversationTemplate, StringTemplate};
use llm_chain::step::Step;
use llm_chain::tools::tools::{BashTool, BashToolInput, BashToolOutput, BashToolError, BingSearch, BingSearchInput, BingSearchOutput, BingSearchError};
use llm_chain::tools::ToolCollection;

// A simple example generating a prompt with some tools.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    use async_trait::async_trait;
    use llm_chain::tools::{Tool, ToolError, ToolDescription};
    use serde::Deserialize;
    use serde::Serialize;
    use thiserror::Error;


    let executor = executor!().unwrap();
    let bing_api_key = std::env::var("BING_API_KEY").unwrap();
    let search_tool = BingSearch::new(bing_api_key);
    multitool!(
        Multitool,
        MultitoolInput,
        MultitoolOutput,
        MultitoolError,
        BashTool,
        BashToolInput,
        BashToolOutput,
        BashToolError,
        BingSearch,
        BingSearchInput,
        BingSearchOutput,
        BingSearchError
    );
    let mut tools = ToolCollection::<Multitool>::new();
    tools.add_tool(search_tool.into());
    tools.add_tool(BashTool::default().into());

    println!("Tools Prompt: {}", tools.to_prompt_template().unwrap().format(&parameters!()).unwrap());

    let agent = Agent::new(
        executor,
        tools,
        EarlyStoppingConfig {
            max_iterations: Some(10),
            max_time_elapsed_seconds: Some(30.0),
        },
    );

    let (res, intermediate_steps) = agent
        .run("Find a file GOAL.txt wherever it is on this machine. It contains a list of names of famous people. Find latest news about those people.")
        .await
        .unwrap();
    println!(
        "Are followup questions needed here: {}",
        agent.build_agent_scratchpad(&intermediate_steps)
    );
    println!(
        "Agent final answer: {}",
        res.return_values.get("output").unwrap()
    );
}
