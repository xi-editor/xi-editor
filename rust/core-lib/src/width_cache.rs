// Copyright 2018 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Cache and utilities for doing width measurement.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

use crate::client::Client;

/// A token which can be used to retrieve an actual width value when the
/// batch request is submitted.
///
/// Internally, it is implemented as an index into the `widths` array.
pub type Token = usize;

/// A measured width, in px units.
type Width = f64;

type StyleId = usize;

pub struct WidthCache {
    /// maps cache key to index within widths
    m: HashMap<WidthCacheKey<'static>, Token>,
    widths: Vec<Width>,
}

#[derive(Eq, PartialEq, Hash)]
struct WidthCacheKey<'a> {
    id: StyleId,
    s: Cow<'a, str>,
}

/// A batched request, so that a number of strings can be measured in a
/// a single RPC.
pub struct WidthBatchReq<'a> {
    cache: &'a mut WidthCache,
    pending_tok: Token,
    req: Vec<WidthReq>,
    req_toks: Vec<Vec<Token>>,
    // maps style id to index into req/req_toks
    req_ids: BTreeMap<StyleId, Token>,
}

/// A request for measuring the widths of strings all of the same style
/// (a request from core to front-end).
#[derive(Serialize, Deserialize)]
pub struct WidthReq {
    pub id: StyleId,
    pub strings: Vec<String>,
}

/// The response for a batch of [`WidthReq`]s.
pub type WidthResponse = Vec<Vec<Width>>;

/// A trait for types that provide width measurement. In the general case this
/// will be provided by the frontend, but alternative implementations might
/// be provided for faster measurement of 'fixed-width' fonts, or for testing.
pub trait WidthMeasure {
    fn measure_width(&self, request: &[WidthReq]) -> Result<WidthResponse, xi_rpc::Error>;
}

impl WidthMeasure for Client {
    fn measure_width(&self, request: &[WidthReq]) -> Result<WidthResponse, xi_rpc::Error> {
        Client::measure_width(self, request)
    }
}

/// A measure in which each codepoint has width of 1.
pub struct CodepointMono;

impl WidthMeasure for CodepointMono {
    /// In which each codepoint has width == 1.
    fn measure_width(&self, request: &[WidthReq]) -> Result<WidthResponse, xi_rpc::Error> {
        Ok(request
            .iter()
            .map(|r| r.strings.iter().map(|s| s.chars().count() as f64).collect())
            .collect())
    }
}

impl WidthCache {
    pub fn new() -> WidthCache {
        WidthCache { m: HashMap::new(), widths: Vec::new() }
    }

    /// Returns the number of items currently in the cache.
    pub(crate) fn len(&self) -> usize {
        self.m.len()
    }

    /// Resolve a previously obtained token into a width value.
    pub fn resolve(&self, tok: Token) -> Width {
        self.widths[tok]
    }

    /// Create a new batch of requests.
    pub fn batch_req(self: &mut WidthCache) -> WidthBatchReq {
        let pending_tok = self.widths.len();
        WidthBatchReq {
            cache: self,
            pending_tok,
            req: Vec::new(),
            req_toks: Vec::new(),
            req_ids: BTreeMap::new(),
        }
    }
}

impl<'a> WidthBatchReq<'a> {
    /// Request measurement of one string/style pair within the batch.
    pub fn request(&mut self, id: StyleId, s: &str) -> Token {
        let key = WidthCacheKey { id, s: Cow::Borrowed(s) };
        if let Some(tok) = self.cache.m.get(&key) {
            return *tok;
        }
        // cache miss, add the request
        let key = WidthCacheKey { id, s: Cow::Owned(s.to_owned()) };
        let req = &mut self.req;
        let req_toks = &mut self.req_toks;
        let id_off = *self.req_ids.entry(id).or_insert_with(|| {
            let id_off = req.len();
            req.push(WidthReq { id, strings: Vec::new() });
            req_toks.push(Vec::new());
            id_off
        });
        // To avoid this second clone, we could potentially do a tricky thing where
        // we extract the strings from the WidthReq. Probably not worth it though.
        req[id_off].strings.push(s.to_owned());
        let tok = self.pending_tok;
        self.cache.m.insert(key, tok);
        self.pending_tok += 1;
        req_toks[id_off].push(tok);
        tok
    }

    /// Resolves pending measurements to concrete widths using the provided [`WidthMeasure`].
    /// On success, the tokens given by `request` will resolve in the cache.
    pub fn resolve_pending<T: WidthMeasure + ?Sized>(
        &mut self,
        handler: &T,
    ) -> Result<(), xi_rpc::Error> {
        // The 0.0 values should all get replaced with actual widths, assuming the
        // shape of the response from the front-end matches that of the request.
        if self.pending_tok > self.cache.widths.len() {
            self.cache.widths.resize(self.pending_tok, 0.0);
            let widths = handler.measure_width(&self.req)?;
            for (w, t) in widths.iter().zip(self.req_toks.iter()) {
                for (width, tok) in w.iter().zip(t.iter()) {
                    self.cache.widths[*tok] = *width;
                }
            }
        }
        Ok(())
    }
}
