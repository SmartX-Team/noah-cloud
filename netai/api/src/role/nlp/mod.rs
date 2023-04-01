pub mod question_answering;

use std::collections::BTreeMap;

use ipis::{
    core::{
        anyhow::{bail, Result},
        ndarray,
    },
    futures::TryFutureExt,
};
use ort::tensor::InputTensor;
use rust_tokenizers::{tokenizer::TruncationStrategy, TokenizedInput};
use serde::{Deserialize, Serialize};

pub(super) struct SolverBase {
    tokenizer: Tokenizer,
}

impl SolverBase {
    async fn load_from_huggingface(repo: &str) -> Result<Self> {
        Tokenizer::load_from_huggingface(repo)
            .map_ok(|tokenizer| Self { tokenizer })
            .await
    }
}

enum Tokenizer {
    DeBERTaV2(::rust_tokenizers::tokenizer::DeBERTaV2Tokenizer),
    Roberta(::rust_tokenizers::tokenizer::RobertaTokenizer),
}

impl Tokenizer {
    async fn load_from_huggingface(repo: &str) -> Result<Self> {
        use crate::models::huggingface as model;

        #[derive(Default, Deserialize)]
        struct Config {
            #[serde(default)]
            model_type: Option<String>,
        }

        #[derive(Default, Deserialize)]
        struct TokenizerConfig {
            #[serde(default)]
            add_prefix_space: bool,
            #[serde(default)]
            do_lower_case: bool,
            #[serde(default)]
            strip_accents: bool,
        }

        let Config { model_type } = model::get_json(repo, "config.json").await?;
        let TokenizerConfig {
            add_prefix_space,
            do_lower_case: lower_case,
            strip_accents,
        } = model::get_json(repo, "tokenizer_config.json").await?;

        match model_type.as_deref() {
            Some("distilbert") => {
                let vocab_path = model::get_file(repo, "vocab.txt").await?;

                ::rust_tokenizers::tokenizer::DeBERTaV2Tokenizer::from_file(
                    vocab_path,
                    lower_case,
                    strip_accents,
                    add_prefix_space,
                )
                .map(Tokenizer::DeBERTaV2)
                .map_err(Into::into)
            }
            Some("roberta") => {
                let vocab_path = model::get_file(repo, "vocab.json").await?;
                let merges_path = model::get_file(repo, "merges.txt").await?;

                ::rust_tokenizers::tokenizer::RobertaTokenizer::from_file(
                    vocab_path,
                    merges_path,
                    lower_case,
                    add_prefix_space,
                )
                .map(Tokenizer::Roberta)
                .map_err(Into::into)
            }
            Some(model_type) => bail!("unsupported model type: {model_type:?}"),
            None => bail!("cannot infer a dynamic model type"),
        }
    }

    fn encode<Input>(
        &self,
        inputs_str: Vec<Input>,
        to_tensor: bool,
    ) -> Result<TokenizedInputs<Input>>
    where
        Input: WordInput,
    {
        match self {
            Self::DeBERTaV2(tokenizer) => Self::encode_with(tokenizer, inputs_str, to_tensor),
            Self::Roberta(tokenizer) => Self::encode_with(tokenizer, inputs_str, to_tensor),
        }
    }

    fn encode_with<Input, T, V>(
        tokenizer: &T,
        inputs_str: Vec<Input>,
        to_tensor: bool,
    ) -> Result<TokenizedInputs<Input>>
    where
        Input: WordInput,
        T: ::rust_tokenizers::tokenizer::Tokenizer<V>,
        V: ::rust_tokenizers::vocab::Vocab,
    {
        fn collect_encode_batch<T>(
            encodings: &[TokenizedInput],
            max_len: usize,
            f: impl Fn(&TokenizedInput) -> &[T],
        ) -> ::ipis::core::anyhow::Result<ndarray::Array<i64, ndarray::Ix2>>
        where
            T: Copy + Into<i64>,
        {
            let arrays: Vec<_> = encodings
                .iter()
                .map(|encoding| {
                    f(encoding)
                        .iter()
                        .copied()
                        .map(Into::into)
                        .collect::<Vec<_>>()
                })
                .map(|mut input| {
                    input.extend([0].repeat(max_len - input.len()));
                    input
                })
                .map(ndarray::Array::from)
                .map(|input| {
                    let length = input.len();
                    input.into_shape((1, length))
                })
                .collect::<Result<_, _>>()?;

            let arrays: Vec<_> = arrays.iter().map(|array| array.view()).collect();
            ndarray::concatenate(ndarray::Axis(0), &arrays).map_err(Into::into)
        }

        let max_len = inputs_str
            .iter()
            .map(WordInput::as_tokenizer_inputs)
            .map(|(text_1, text_2)| text_1.len().max(text_2.map(|e| e.len()).unwrap_or(0)))
            .max()
            .unwrap_or(0);

        let inputs_1: Vec<_> = inputs_str
            .iter()
            .map(WordInput::as_tokenizer_input_1)
            .collect();
        let inputs_2: Vec<_> = inputs_str
            .iter()
            .filter_map(WordInput::as_tokenizer_input_2)
            .collect();

        if !inputs_2.is_empty() && inputs_1.len() != inputs_2.len() {
            bail!("failed to parse the text pairs");
        }

        let encodings = if inputs_2.is_empty() {
            tokenizer.encode_list(&inputs_1, max_len, &TruncationStrategy::LongestFirst, 0)
        } else {
            let inputs_pair: Vec<_> = inputs_1.into_iter().zip(inputs_2.into_iter()).collect();

            tokenizer.encode_pair_list(&inputs_pair, max_len, &TruncationStrategy::LongestFirst, 0)
        };
        let input_lens: Vec<_> = encodings
            .iter()
            .map(|encoding| encoding.token_ids.len())
            .collect();
        let max_len = input_lens.iter().max().copied().unwrap_or(0);

        let input_ids = collect_encode_batch(&encodings, max_len, |encoding| &encoding.token_ids)?;

        let inputs = if to_tensor {
            let attention_mask = ndarray::Array::ones(input_ids.dim());
            let token_type_ids =
                collect_encode_batch(&encodings, max_len, |encoding| &encoding.segment_ids)?;

            vec![
                (
                    "input_ids".to_string(),
                    InputTensor::Int64Tensor(input_ids.clone().into_dyn()),
                ),
                (
                    "attention_mask".to_string(),
                    InputTensor::Int64Tensor(attention_mask.into_dyn()),
                ),
                (
                    "token_type_ids".to_string(),
                    InputTensor::Int64Tensor(token_type_ids.into_dyn()),
                ),
            ]
            .into_iter()
            .collect()
        } else {
            Default::default()
        };

        Ok(TokenizedInputs {
            input_ids,
            inputs,
            inputs_str,
        })
    }

    fn decode(&self, token_ids: &[i64]) -> String {
        let skip_special_tokens = true;
        let clean_up_tokenization_spaces = true;

        match self {
            Self::DeBERTaV2(tokenizer) => Self::decode_with(
                tokenizer,
                token_ids,
                skip_special_tokens,
                clean_up_tokenization_spaces,
            ),
            Self::Roberta(tokenizer) => Self::decode_with(
                tokenizer,
                token_ids,
                skip_special_tokens,
                clean_up_tokenization_spaces,
            ),
        }
    }

    fn decode_with<T, V>(
        tokenizer: &T,
        token_ids: &[i64],
        skip_special_tokens: bool,
        clean_up_tokenization_spaces: bool,
    ) -> String
    where
        T: ::rust_tokenizers::tokenizer::Tokenizer<V>,
        V: ::rust_tokenizers::vocab::Vocab,
    {
        tokenizer
            .decode(token_ids, skip_special_tokens, clean_up_tokenization_spaces)
            .trim()
            .to_string()
    }
}

trait WordInput {
    fn as_tokenizer_inputs(&self) -> (&str, Option<&str>);

    fn as_tokenizer_input_1(&self) -> &str;

    fn as_tokenizer_input_2(&self) -> Option<&str>;
}

#[derive(Serialize)]
pub struct QuestionWordInput {
    pub context: String,
    pub question: String,
}

impl WordInput for QuestionWordInput {
    fn as_tokenizer_inputs(&self) -> (&str, Option<&str>) {
        (&self.question, Some(&self.context))
    }

    fn as_tokenizer_input_1(&self) -> &str {
        &self.question
    }

    fn as_tokenizer_input_2(&self) -> Option<&str> {
        Some(&self.context)
    }
}

struct TokenizedInputs<Input> {
    input_ids: ndarray::Array<i64, ndarray::Ix2>,
    inputs: BTreeMap<String, InputTensor>,
    inputs_str: Vec<Input>,
}
