# Built with tuika

Applications whose terminal UI is built on tuika — what they are, and what the
toolkit looks like once it is carrying a real product.

Building something on tuika? Open a PR adding it here.

## yolop

A terminal coding agent that plans, edits, runs, and verifies code in your
repository: persistent sessions, agent skills, MCP servers, and editor
integration over the Agent Client Protocol. Its full-screen renderer is built on
tuika — the transcript, the streaming markdown and highlighted code, the tool
cards, the composer, and the status footer are all tuika components under
tuika's layout and host loop.

[github.com/everruns/yolop](https://github.com/everruns/yolop) ·
[crates.io](https://crates.io/crates/yolop)

<img src="showcases/yolop.gif" width="880" alt="yolop demo: a question typed into the composer, a tool call listing the examples directory, then a streamed markdown answer with a highlighted bash code block.">

The recorded session is deterministic and offline — the model is a local
[LLMSim](#llmsim) replaying a scripted turn — so the recording shows the real
agent loop without a provider key.

## LLMSim

An LLM traffic simulator: a local server that speaks the OpenAI, OpenResponses,
and Anthropic APIs with realistic streaming, latency profiles, token accounting,
and injected failures, so load tests and CI runs never touch a real model. Its
`serve --tui` stats dashboard is a tuika screen — the panel grid is flexbox
layout, and the counters, sparklines, and model distribution redraw live while
requests are in flight.

[github.com/chaliy/llmsim](https://github.com/chaliy/llmsim) ·
[crates.io](https://crates.io/crates/llmsim)

<img src="showcases/llmsim.gif" width="880" alt="LLMSim demo: the stats dashboard under live traffic, with request, token, latency, and error panels updating alongside RPS and tokens-per-second sparklines and a model distribution chart.">

The dashboard above is under ~5 requests/second across four models, with rate
limit and server-error injection turned on.
