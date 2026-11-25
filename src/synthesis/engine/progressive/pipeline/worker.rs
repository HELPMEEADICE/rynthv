//! The browser Worker advances one vocoder batch on its own WASM stack.

use super::{Input, Output};
use crate::synthesis::{
    inference::Sequence,
    stages::vocoder::{VocoderStage, VocoderStream, VoiceControl},
    voice::{PreparedTree, Result},
};
use web_time::Instant;

pub(in super::super) struct Pipeline<'tree> {
    stream: Option<VocoderStream<'tree>>,
    output: Option<Output>,
    setup_ms: f64,
}

impl<'tree> Pipeline<'tree> {
    pub fn new(tree: &'tree PreparedTree) -> Result<Self> {
        let start = Instant::now();
        let stream = VocoderStage::prepare(tree)?.into_stream()?;
        Ok(Self {
            stream: Some(stream),
            output: None,
            setup_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    pub fn can_submit(&self) -> bool {
        self.output.is_none() && self.stream.is_some()
    }

    pub fn submit(
        &mut self,
        physical: Sequence,
        controls: Option<Sequence>,
        voice_controls: Option<Vec<VoiceControl>>,
        natural_final: bool,
    ) -> Result<()> {
        let input = Input::new(
            self.can_submit(),
            physical,
            controls,
            voice_controls,
            natural_final,
        )?;
        let output = input.process(
            self.stream.as_mut().expect("open vocoder stream"),
            std::mem::take(&mut self.setup_ms),
            #[cfg(feature = "synth-profile")]
            None,
        )?;
        if natural_final {
            self.stream = None;
        }
        self.output = Some(output);
        Ok(())
    }

    pub fn poll(&mut self) -> Option<Output> {
        self.output.take()
    }

    pub fn stop(&mut self) {
        self.stream = None;
        self.output = None;
    }
}
