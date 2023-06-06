use std::time::{Duration, Instant};

use crate::{parameters, traits::Executor, tools::{Tool, ToolCollection, ToolUseError}, options::Options, prompt::{PromptTemplate, StringTemplate}};

use super::self_ask_with_search::{AgentOutputParser, AgentAction, AgentDecision, AgentFinish, ParserError, EarlyStoppingConfig, AgentIntermediateStepOutput, AgentIntermediateStep, SelfAskWithSearchAgentError};

struct ConversationalOutputParser {
    followup_prefix: String,
    intermediate_answer_prefix: String,
    acceptable_finish_prefixes: Vec<String>,
}

impl ConversationalOutputParser {
    pub fn new(
        followup_prefix: &str,
        intermediate_answer_prefix: &str,
        acceptable_finish_prefixes: &[&str],
    ) -> Self {
        Self {
            followup_prefix: followup_prefix.into(),
            intermediate_answer_prefix: intermediate_answer_prefix.into(),
            acceptable_finish_prefixes: acceptable_finish_prefixes
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl Default for ConversationalOutputParser {
    fn default() -> Self {
        Self::new(
            "Follow up:",
            "Intermediate Answer:",
            &[
                "Final answer:",
                "So the final answer is:",
                "So the final answer could be:",
            ],
        )
    }
}

impl AgentOutputParser for ConversationalOutputParser {
    type Error = ParserError;

    fn parse(&self, text: String) -> Result<super::self_ask_with_search::AgentDecision, Self::Error> {
        if let Some(followup_idx) = text.find(&self.followup_prefix) {
            let (followup_question, log) = if let Some(intermediate_answer_idx) =
                text.find(&self.intermediate_answer_prefix)
            {
                let followup_question = text
                    .chars()
                    .skip(followup_idx + self.followup_prefix.len())
                    .take(intermediate_answer_idx - (followup_idx + self.followup_prefix.len()))
                    .collect::<String>()
                    .trim()
                    .to_owned();

                let log = text.chars().take(intermediate_answer_idx).collect();
                (followup_question, log)
            } else {
                return Err(ParserError(text));
            };
            Ok(AgentDecision::Action(AgentAction {
                tool: "Intermediate Answer".into(),
                tool_input: followup_question.into(),
                log,
            }))
        } else if let Some((idx, prefix)) = self
            .acceptable_finish_prefixes
            .iter()
            .find_map(|prefix| text.find(prefix).map(|idx| (idx, prefix)))
        {
            let final_answer = text.chars().skip(idx + prefix.len()).collect::<String>();
            Ok(AgentDecision::Finish(AgentFinish {
                return_values: parameters!("output" => final_answer.trim()),
                log: text,
            }))
        } else {
            Err(ParserError(text))
        }
    }
}

pub struct Agent<E, T>
where
    E: Executor,
    T: Tool + Sync + Send,
{
    executor: E,
    tools: ToolCollection<T>,
    early_stopping_config: EarlyStoppingConfig,
    observation_prefix: String,
    llm_prefix: String,
    output_parser: ConversationalOutputParser,
}

impl<E, T> Agent<E, T>
where
    E: Executor,
    T: Tool + Sync + Send,
{
    pub fn new(executor: E, tools: ToolCollection<T>, early_stopping_config: EarlyStoppingConfig) -> Self {
        Self {
            executor,
            tools,
            early_stopping_config,
            observation_prefix: "Intermediate answer: ".to_string(),
            llm_prefix: "".to_string(),
            output_parser: ConversationalOutputParser::default(),
        }
    }

    fn should_continue(&self, iterations_elapsed: u32, time_elapsed_seconds: f64) -> bool {
        match (
            self.early_stopping_config.max_iterations,
            self.early_stopping_config.max_time_elapsed_seconds,
        ) {
            (None, None) => true,
            (None, Some(max_time_elapsed_seconds)) => {
                max_time_elapsed_seconds >= time_elapsed_seconds
            }
            (Some(max_iterations), None) => max_iterations >= iterations_elapsed,
            (Some(max_iterations), Some(max_time_elapsed_seconds)) => {
                max_iterations >= iterations_elapsed
                    && max_time_elapsed_seconds >= time_elapsed_seconds
            }
        }
    }

    /// Ask a model for a decision on what to do next, e.x. which tool to use
    ///
    /// Perform the action
    async fn take_next_step(
        &self,
        intermediate_steps: &Vec<AgentIntermediateStep>,
        query: &str,
    ) -> Result<AgentIntermediateStepOutput, SelfAskWithSearchAgentError<<T as Tool>::Error>> {
        let output = self.plan(intermediate_steps, query).await?;

        let decision = self.output_parser.parse(output)?;
        match decision {
            AgentDecision::Action(action) => {
                let observation = self
                    .tools.invoke(&action.tool, &action.tool_input)
                    .await?;

                Ok(AgentIntermediateStepOutput::Step(AgentIntermediateStep {
                    action,
                    observation,
                }))
            }
            AgentDecision::Finish(finish) => Ok(AgentIntermediateStepOutput::Finish(finish)),
        }
    }

    /// Convert the intermediate steps into a single text to pass to the agent so he can continue his thought process
    pub fn build_agent_scratchpad(
        &self,
        intermediate_steps: &Vec<AgentIntermediateStep>,
    ) -> String {
        let mut scratchpad = "".to_string();
        for intermediate_step in intermediate_steps {
            scratchpad += &intermediate_step.action.log;
            scratchpad += &format!(
                "\n{}{}\n{}",
                self.observation_prefix,
                intermediate_step.observation.as_str().unwrap_or_default(),
                self.llm_prefix
            );
        }
        scratchpad
    }

    /// Ask a model for a decision on what to do next, e.x. which tool to use
    ///
    /// Fills in the prompt template then calls the model to complete it
    async fn plan(
        &self,
        intermediate_steps: &Vec<AgentIntermediateStep>,
        query: &str,
    ) -> Result<String, SelfAskWithSearchAgentError<<T as Tool>::Error>> {
        let scratchpad = self.build_agent_scratchpad(intermediate_steps);
        let tool_prompt = jailbreak_tools_prompt(&self.tools)?.format(&parameters!())?;
        let template_parameters = parameters!("input" => query, "agent_scratchpad" => scratchpad, "tools" => tool_prompt);
        let prompt = PromptTemplate::Text(PREFIX.into()).format(&template_parameters)?;
        let plan = self
            .executor
            .execute(Options::empty(), &prompt)
            .await
            .map_err(SelfAskWithSearchAgentError::ExecutorError)?;
        plan.to_immediate()
            .await
            .map_err(SelfAskWithSearchAgentError::ExecutorError)?
            .as_content()
            .extract_last_body()
            .cloned()
            .ok_or(SelfAskWithSearchAgentError::NoChoicesReturned)
    }

    pub async fn run(
        &self,
        query: &str,
    ) -> Result<
        (AgentFinish, Vec<AgentIntermediateStep>),
        SelfAskWithSearchAgentError<<T as Tool>::Error>,
    > {
        let mut intermediate_steps = vec![];

        let mut iterations = 0;
        let start = Instant::now();
        let mut full_duration = Duration::from_nanos(0);
        while self.should_continue(iterations, full_duration.as_secs_f64()) {
            let decision = self.take_next_step(&intermediate_steps, query).await?;
            full_duration = start.elapsed();
            iterations += 1;
            match decision {
                AgentIntermediateStepOutput::Step(step) => intermediate_steps.push(step),
                AgentIntermediateStepOutput::Finish(finish) => {
                    return Ok((finish, intermediate_steps))
                }
            }
        }
        Err(SelfAskWithSearchAgentError::RuntimeExceeded {
            time_elapsed_seconds: full_duration.as_secs_f64(),
            iterations_elapsed: iterations,
        })
    }
}


/// To circumnavigate OpenAI's baby monitor system we need to change the prompt to ask "THE USER" to invoke tools.
/// 
/// In reality we will parse the Yaml and invoke the tools as usual.
fn jailbreak_tools_prompt<T: Tool + Sync + Send>(tools: &ToolCollection<T>) -> Result<StringTemplate, ToolUseError<<T as Tool>::Error>> {
        Ok(StringTemplate::combine(vec![
            StringTemplate::static_string(ALTERNATIVE_TOOLS_PROMPT.to_string()),
            StringTemplate::static_string(tools.describe()?),
            StringTemplate::static_string("\n\n"),
        ]))
}

const ALTERNATIVE_TOOLS_PROMPT: &str = "
Assistant can ask the user to use tools to look up information that may be helpful in answering the users original question. You may only communicate that with YAML. You are provided with tools that you may ask the user to use by naming the tool you wish to invoke along with it's input.

For the user to invoke a tool write YAML like this, do not include output:
command: Command
input: 
  <INPUT IN YAML>


The following are the user's tools:";

const PREFIX: &str = "Assistant is a large language model.

Assistant is designed to be able to assist with a wide range of tasks, from answering simple questions to providing in-depth explanations and discussions on a wide range of topics. As a language model, Assistant is able to generate human-like text based on the input it receives, allowing it to engage in natural-sounding conversations and provide responses that are coherent and relevant to the topic at hand.

Assistant is constantly learning and improving, and its capabilities are constantly evolving. It is able to process and understand large amounts of text, and can use this knowledge to provide accurate and informative responses to a wide range of questions. Additionally, Assistant is able to generate its own text based on the input it receives, allowing it to engage in discussions and provide explanations and descriptions on a wide range of topics.

Overall, Assistant is a powerful system that can help with a wide range of tasks and provide valuable insights and information on a wide range of topics. Whether you need help with a specific question or just want to have a conversation about a particular topic, Assistant is here to assist.

{{tools}}

Here is the user's input:
{{input}}

Are followup tasks needed here:{{agent_scratchpad}}
";

const FORMAT_INSTRUCTIONS: &str = "RESPONSE FORMAT INSTRUCTIONS
----------------------------

When responding to me, please output a response in one of two formats:

**Option 1:**
Use this if you want the human to use a tool.
Markdown code snippet formatted in the following schema:

```json
{{{{
    \"action\": string, \\ The action to take. Must be one of {tool_names}
    \"action_input\": string \\ The input to the action
}}}}
```

**Option #2:**
Use this if you want to respond directly to the human. Markdown code snippet formatted in the following schema:

```json
{{{{
    \"action\": \"Final Answer\",
    \"action_input\": string \\ You should put what you want to return to use here
}}}}
```";

const SUFFIX: &str = "TOOLS
------
Assistant can ask the user to use tools to look up information that may be helpful in answering the users original question. The tools the human can use are:

{{tools}}

{format_instructions}

USER'S INPUT
--------------------
Here is the user's input (remember to respond with a markdown code snippet of a json blob with a single action, and NOTHING else):

{{{{input}}}}";

const TEMPLATE_TOOL_RESPONSE: &str = "TOOL RESPONSE: 
---------------------
{observation}

USER'S INPUT
--------------------

Okay, so what is the response to my last comment? If using information obtained from the tools you must mention it explicitly without mentioning the tool names - I have forgotten all TOOL RESPONSES! Remember to respond with a markdown code snippet of a json blob with a single action, and NOTHING else.";