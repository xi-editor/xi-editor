// Copyright 2016 The xi-editor Authors.
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

// A tool for generating random strings and result of the ICU line break
// iterator.

#include <unicode/brkiter.h>

#include <iostream>
#include <random>
#include <vector>

using std::cout;
using std::endl;
using std::rand;
using std::string;
using std::vector;
using icu::BreakIterator;
using icu::UnicodeString;
using icu::StringPiece;

void push_utf8(string* buf, uint32_t cp) {
    if (cp < 0x80) {
        *buf += cp;
    } else if (cp < 0x800) {
        *buf += 0xc0 | (cp >> 6);
        *buf += 0x80 | (cp & 0x3f);
    } else if (cp < 0x10000) {
        *buf += 0xe0 | (cp >> 12);
        *buf += 0x80 | ((cp >> 6) & 0x3f);
        *buf += 0x80 | (cp & 0x3f);
    } else {
        *buf += 0xf0 | (cp >> 18);
        *buf += 0x80 | ((cp >> 12) & 0x3f);
        *buf += 0x80 | ((cp >> 6) & 0x3f);
        *buf += 0x80 | (cp & 0x3f);
    }
}

string randstring(vector<uint32_t>* codepoints) {
    static std::default_random_engine generator;
    static std::exponential_distribution<double> expd(1.0);
    static std::uniform_real_distribution<double> unif;
    string result;
    uint32_t len = 1 + (uint32_t)(10 * expd(generator));
    while (result.size() < len) {
        double kind = unif(generator);
        double lo = 0x20, hi;
        if (kind < 0.01) {
            lo = 0;
            hi = 0x20;
        } else if (kind < 0.5) {
            hi = 0x7f;
        } else if (kind < 0.8) {
            hi = 0x800;
        } else if (kind < 0.95) {
            hi = 0x10000;
        } else {
            hi = 0x110000;
        }
        uint32_t cp = (uint32_t)(lo + (hi - lo) * unif(generator));
        if (cp < 0xd800 || (0xe000 <= cp && cp < 0x110000)) {
            codepoints->push_back(cp);
            push_utf8(&result, cp);
        }
    }
    return result;
}

void report_string(const string& s, const vector<size_t>& breaks,
        const vector<uint32_t>& codepoints) {
    size_t bks_ix = 0;
    size_t utf8_ix = 0;
    cout << "×";
    for (size_t i = 0; i < codepoints.size(); i++) {
        uint8_t b = s[utf8_ix];
        size_t cp_len = 1;
        if (b >= 0xf0) {
            cp_len = 4;
        } else if (b >= 0xe0) {
            cp_len = 3;
        } else if (b >= 0xc0) {
            cp_len = 2;
        }
        utf8_ix += cp_len;
        cout << " " << std::hex << codepoints[i];
        if (breaks[bks_ix] == utf8_ix) {
            cout << " ÷";
            bks_ix++;
        } else {
            cout << " ×";
        }
    }
    cout << endl;
}

int main(int argc, char** argv) {
    int niter = 100;
    if (argc == 2) niter = atoi(argv[1]);
    UText ut = UTEXT_INITIALIZER;
    UErrorCode status = U_ZERO_ERROR;
    BreakIterator* bi = BreakIterator::createLineInstance(Locale(), status);
    vector<size_t> breaks;
    vector<uint32_t> codepoints;
    for (int i = 0; i < niter; i++) {
        codepoints.clear();
        breaks.clear();
        string s = randstring(&codepoints);
        utext_openUTF8(&ut, s.data(), s.size(), &status);
        bi->setText(&ut, status);
        bool first = true;
        while (true) {
            int32_t i = bi->next();
            if (i == BreakIterator::DONE) {
                break;
            }
            breaks.push_back(i);
            first = false;
        }
        report_string(s, breaks, codepoints);
        utext_close(&ut);
    }
    return 0;
}
