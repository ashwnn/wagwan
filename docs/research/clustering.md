# News story clustering, near-duplicates, novelty, and LLM summarization: research notes

Date: 2026-10-22. Web research only. Items marked **UNVERIFIED** were not confirmed in a source I read; they are either my own recommendations or recalled details that need checking.

## Corrections to common assumptions

- Das et al. 2007 ("Google News Personalization", WWW 2007) uses MinHash/LSH to cluster **users** by click sets, not news stories. Stories are scored from user-cluster click counts. Google's production story-clustering internals are not public, so "Google News style" design details are **UNVERIFIED**.
- The Bge FAQ and Petrovic et al. thresholds are for different distance conventions (cosine similarity vs. cosine distance). Check which one a source reports before reusing a number.

## 1. Incremental / online story clustering

**Miranda et al. 2018, "Multilingual Clustering of Streaming News" (EMNLP 2018)**
- Source: https://arxiv.org/abs/1809.00540 (ACL: https://aclanthology.org/D18-1483/)
- Online single-pass method: each incoming document joins the best monolingual cluster or starts a new one, using a threshold tau. Similarity is cosine over TF-IDF subvectors plus three timestamp features compared against the cluster's newest, average, and oldest timestamps.
- Time decay: Gaussian f(t) = exp(-(t-mu)^2 / 2 sigma^2), mu = 0, sigma = 72 hours.
- Centroid = average of member representations. Monolingual assignments are immutable; they are never revised after the fact.
- Tau is tuned per language on a dev set; the paper does not report the tau values.
- Results (pairwise F1, monolingual): English 94.1 with timestamps vs. 79.0 for CluStream baseline; German 97.1 vs. 89.7; Spanish 94.2 vs. 78.1.
- Adding the timestamp features is the paper's main anti-drift lever ("help disambiguate clustering decisions").
- Merge alternative: a learned SVM-merge decision beat tau-search (English F1 94.1 vs. 82.8).
- Dataset: see section 3 (cluster-linking data from Rupnik et al. 2016).

**Petrovic, Osborne, Lavrenko 2010, "Streaming First Story Detection with application to Twitter" (NAACL 2010)**
- Source: https://preview.aclanthology.org/fix_video/N10-1021.pdf (ACL: https://aclanthology.org/N10-1021/)
- Random-hyperplane LSH (Charikar), k = 13 bits per key; number of tables L chosen so a neighbor within cosine distance 0.2 is missed with probability below 2.5%.
- Novelty score = minimum cosine distance to previously seen documents. No fixed cutoff for "first story"; scores are ranked and swept.
- Variance reduction: compare each flagged document against the 2,000 most recent documents.
- Bounded streaming: each hash bucket is capped in size; each new document is compared to at most 3L candidates.
- Threading threshold (cosine distance) t = 0.5, stable for 0.5 to 0.6.
- Datasets: TDT5 English (221,306 docs, April to September 2003); Twitter (about 163.5 million tweets, 2009).
- Speed: about 2 hours vs. 28 hours for the UMass baseline on TDT5.

**Recent LLM-era / embedding-era systems**
- "Real-time News Story Identification" (arXiv 2508.08272): https://arxiv.org/pdf/2508.08272 . Embeds title plus body (not lede only) with BGE-M3, which was the best of the models they tested (TF-IDF, DistilBERT, TSDAE, multilingual-e5-large-instruct, paraphrase-MiniLM-L6-v2, and others). Each story is assumed to span at most 10 days; a fading factor makes articles about 10 days old near-zero weight. Dataset: about 6,400 Slovene articles grouped into 4,028 stories (most stories are singletons). Online methods trailed offline clustering substantially (for example, DBSTREAM 0.162 vs. k-means 0.838 on AMI). The paper gives no tau values for its online method. Treat this as a useful time-window precedent, not a tuned threshold.

**Story chains (Shahaf and Guestrin, "Connecting the Dots Between News Articles", KDD 2010)**
- Relevant to chaining/metro-map style linking of stories over time. I did not read the paper; method details are **UNVERIFIED**. Search entry point: https://www.ijcai.org/Proceedings/11/Papers/455.pdf is a related follow-up (not read).

**Chaining and drift, summary of mitigations (partly sourced, partly recommendation)**
- Compare an article to a cluster **centroid** (or average-link to members), not single-link to any member. Single-link is what produces chains. (Miranda uses centroids; recommendation for the single-link warning is my own, **UNVERIFIED** as a quantified claim.)
- Time decay on similarity or a hard time window (Miranda sigma = 72h; 2508.08272 uses a ~10-day story span).
- Never merge two existing clusters after the fact unless you have a separate gate; Miranda keeps assignments immutable.
- Entity/date gate: require overlap in named entities or a date proximity in addition to embedding similarity before joining (**UNVERIFIED** as a measured gain; it is the standard fix in TDT-style systems).
- Close inactive clusters (recommendation: 3 days without new members, **UNVERIFIED**).

**Title-only vs. title+lede vs. full text for event-level similarity**
- No source I read directly compares title-only vs. lede vs. full-text embeddings for same-event detection. **UNVERIFIED** for event-level comparisons.
- Evidence available: 2508.08272 embeds title plus full body. Adding named entities to BGE-M3 features lowered clustering AMI (0.838 to 0.791), which argues against bolting on extra features without testing.
- Practical constraint: all-MiniLM-L6-v2 truncates at 256 word pieces (verified, see section 2), so a lede-length input (title plus first ~100 to 150 words) is what fits. Recommendation: title + lede for the embedding, full text only for the duplicate and novelty steps.

## 2. Cosine thresholds for "same event" with small models

**BGE (bge-*-v1.5) FAQ (verified on the bge-large-en-v1.5 model card, the same FAQ family as bge-small-en-v1.5):**
- Source: https://huggingface.co/BAAI/bge-large-en-v1.5 and https://huggingface.co/BAAI/bge-small-en-v1.5
- "the similarity distribution of the current BGE model is about in the interval [0.6, 1]."
- "a similarity score greater than 0.5 does not indicate that the two sentences are similar."
- "what matters is the relative order of the scores, not the absolute value."
- "please select an appropriate similarity threshold based on the similarity distribution on your data."
- v1.5 was released to "alleviate the issue of the similarity distribution" (compression). Implication: calibrate per model and per dataset; do not copy thresholds from other models.

**all-MiniLM-L6-v2 (verified):**
- Source: https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- 384-dimensional output; input truncated at 256 word pieces by default; Apache-2.0 license.
- No published same-event cosine threshold found. **UNVERIFIED** values I have seen in practice are roughly 0.6 to 0.8, but I did not find a source.

**sentence-transformers community_detection:** a function with a `threshold` parameter. I could not fetch its default; the commonly cited default is 0.75. **UNVERIFIED.** Reference: https://sbert.net/docs/package_reference/util.html

**Petrovic 2010 (distance, not similarity):** threading threshold cosine distance 0.5 to 0.6 (= cosine similarity 0.4 to 0.5) on TDT5/Twitter with LSH. Different corpus, different model era; use only as a sanity bound.

**TREC Novelty track similarity threshold:** 0.4 (see section 5); lexical, not embedding.

**Recommendation (not sourced):** because bge-small scores are compressed into roughly [0.6, 1], fit thresholds on the labeled pairs (see section 3) and express the rule relative to the background distribution: sample random article pairs from your feed mix, compute mean and standard deviation of their similarity, and set tau_join at a fixed number of standard deviations above that background. Starting guess for bge-small on title+lede: 0.80 to 0.85, **UNVERIFIED**, to be replaced by calibration.

## 3. Datasets for tuning and evaluation

| Dataset | What it labels | Size (as reported) | License | Link |
|---|---|---|---|---|
| SemEval-2022 Task 8 (multilingual news article similarity) | Pairwise similarity on 7 aspects, including "overall news story" | about 10,000 pairs; 18 language combinations; articles from H1 2020 | CC BY 4.0 (verified on Zenodo) | https://zenodo.org/records/6507872 (DOI 10.5281/zenodo.6507872); paper https://aclanthology.org/2022.semeval-1.155/ |
| Rupnik et al. 2016 cluster-linking data, as used by Miranda et al. 2018 | Document-to-cluster linking; cluster IDs | English train 12,233 docs / 593 clusters; English test 8,726 / 222; German and Spanish also | Not stated in the paper; **UNVERIFIED** | Original: https://github.com/rupnikj/jair_paper ; processed: https://github.com/priberam/news-clustering |
| TDT5 / TDT2-3 (NIST Topic Detection and Tracking) | Story and event labels | TDT5 English: 221,306 docs (Apr to Sep 2003) | LDC licensed (paid or membership); exact terms **UNVERIFIED** | https://www.ldc.upenn.edu/sites/default/files/tdt2000-tdt3-corpus.pdf |
| Multi-News (Fabbri et al. 2019) | Multi-document summaries, not event labels | multi-doc summary pairs | **UNVERIFIED** | https://huggingface.co/datasets/alexfabbri/multi_news |
| MIND (Microsoft news recommendation) | User clicks, not event labels | **UNVERIFIED** | **UNVERIFIED** (Microsoft research license expected) | Not verified; poor fit for event clustering |

Notes:
- SemEval gives pairwise labels, which suit threshold tuning directly. Use the "overall news story" aspect.
- Rupnik-based data gives cluster labels, which suit end-to-end evaluation (pairwise P/R/F1 as in Miranda).
- For your feeds, consider weak labels: pairs of articles that link to the same primary source or wire ID are likely the same event. Use a small hand-labeled sample to validate.

## 4. Near-duplicate detection for syndicated / wire copy

**Lee et al. 2022, "Deduplicating Training Data Makes Language Models Better" (ACL 2022)**
- Source: https://arxiv.org/pdf/2107.06499 ; code: https://github.com/google-research/deduplicate-text-datasets (license not stated in the paper; **UNVERIFIED**).
- 5-gram word shingles over space-tokenized text; MinHash signature size 9,000; banding parameters b = 20, r = 450 (the paper's appendix and main text describe banding slightly differently).
- Candidate pairs with Jaccard above 0.8 go to an edit-similarity check; edit similarity above 0.8 declares a duplicate.

**Henzinger 2006, "Finding Near-Duplicate Web Pages: A Large-Scale Evaluation of Algorithms" (SIGIR 2006)**
- Source: https://infoscience.epfl.ch/record/99373/files/Henzinger06.pdf?version=1
- Broder-style shingling: 8-token shingles; 84 min-values grouped into 6 supershingles; near-duplicate if at least 2 supershingles match. Precision 0.38 on a 1,910-pair sample.
- Simhash (Charikar), 384-bit, near-duplicate if at least 372 bits agree. Precision 0.50 on a 1,872-pair sample.
- Combined (shingle candidates filtered by simhash >= 355): precision 0.79, keeping about 79% of shingle recall.
- Key finding: both methods fail mainly on same-site near-duplicates because of **boilerplate** (navigation, footers, related-article blocks). Strip boilerplate before hashing.

**Recommended defaults (recommendation; the numbers come from the sources above, the combination is mine):**
- Normalize: lowercase, strip punctuation, collapse whitespace, remove boilerplate (use readability extraction).
- Shingles: 5 words (Lee et al.) or 8 tokens (Henzinger). Start with 5 words for syndicated copy.
- MinHash with 128 permutations and LSH banding tuned for Jaccard 0.8 (**UNVERIFIED** band count; compute with the standard LSH threshold formula).
- Verify candidates with Jaccard >= 0.8 (Lee et al.).
- For syndicated copy that adds a local paragraph, Jaccard drops below 0.8 even though one article contains the other. Add a containment check: |A intersect B| / min(|A|, |B|) >= 0.8 (**UNVERIFIED** threshold; my recommendation).

## 5. Information novelty / unique facts per article

**TREC Novelty track 2002 to 2004**
- Source: https://arxiv.org/pdf/cs/0510054 (also TREC pages: https://pages.nist.gov/trec-browser/trec13/novelty/overview/ )
- Task: sentence-level, query-specific; a sentence is redundant or novel (Boolean). Chronological order; remove sentences that add nothing to earlier sentences.
- Redundancy rates: nv03 about 34.1%, nv04 about 53.7%, nvyiz about 10.8% per topic.
- Methods were overlap-based (word overlap with earlier sentences). Overlap thresholds 0.7 (nv03, nv04) and 0.8 (nvyiz). Similarity threshold 0.4.
- Metric warning: the F-measure can rise when a low-precision set is added, so report precision and recall separately.

**Petrovic et al. 2010 (Twitter):** ranking by entropy plus unique users, with an entropy threshold of 3.5, scored AP 34.0 vs. 16.5 for baseline on Twitter event detection. Entropy of the term distribution is a cheap novelty/informativeness proxy.

**Proposed cheap heuristic (my design, not sourced as a method):**
1. Extract "fact tokens": normalized numbers (with units; treat "3.2 million" and "3,200,000" as equal), dates and times, capitalized multi-word spans, and quoted strings with speaker names.
2. For each article in a cluster, compute the fact set not present in the union of the other members' fact sets, ordered by time (earlier articles are the baseline; later articles are measured against earlier ones).
3. Unique-fact ratio = new facts / total facts. Use it to rank articles within a story, not as an absolute truth score.
4. Caveats (**UNVERIFIED** impact): copycat rewrites change number formatting, entity names change in translation, and quotes are often paraphrased. Normalize before comparing.

**Redundancy-aware ranking:** Maximal Marginal Relevance (Carbonell and Goldstein 1998) is the usual reference for penalizing redundancy when selecting items. **UNVERIFIED** (not fetched).

## 6. LLM multi-document summarization practices

**ALCE: Enabling Large Language Models to Generate Text with Citations (Gao et al., EMNLP 2023)**
- Source: https://arxiv.org/pdf/2305.14627 ; code: https://github.com/fallcat/ALCE
- Citation recall: an NLI model judges whether each statement is fully supported by its cited passages. Citation precision: each citation is relevant (not superfluous).
- Finding: around 50% of ChatGPT and GPT-4 generations were not fully supported by the cited passages (ELI5). ChatGPT with top-5 passages: citation recall 51.1, precision 50.0.
- Implication: citations are not reliable by default; verify them.

**SummaC (Laban et al., TACL 2022)**
- Source: https://arxiv.org/pdf/2111.09525
- Sentence-level NLI: split the source into sentences and the summary into sentences; score every pair with NLI; aggregate (SummaC-ZS takes the max entailment per summary sentence, then averages).
- Document-level NLI was poor (MNLI-doc 61.3% balanced accuracy on the benchmark). SummaC-Conv reaches 74.4%.
- Lightweight verification: run a small NLI cross-encoder (model choice **UNVERIFIED**) on each generated claim vs. each source sentence in the cited article(s); flag claims with no entailing sentence.

**Practical prompt and pipeline recommendations (my recommendations; the sources above support the verification step):**
- Give each source a stable ID ([S1], [S2], ...) with publisher, publish time, and URL. Require each sentence in the output to cite one or more IDs.
- Instruct the model to use only the provided sources and to say so when a claim cannot be supported.
- Separate "agreed" facts (supported by 2 or more sources) from "reported by one source" and from "conflicting" claims. For conflicts, present each side with its attribution ("According to [S2]... [S4] reports...") instead of picking one.
- Deterministic checks that need no model: every number, date, and quoted string in the output must appear in at least one cited source after normalization. Reject or flag any that do not.
- Keep the LLM out of the clustering decision; use it only for summarizing a cluster that already exists.

## Recommended defaults

| Parameter | Default | Basis |
|---|---|---|
| Time window | Sigma 72 hours for time decay; hard story span cap of 10 days; close clusters after 3 days idle | Miranda et al. 2018 (72h); arXiv 2508.08272 (10 days); idle cutoff UNVERIFIED |
| Assignment rule | Join the best active cluster by similarity to the centroid if above tau_join, else open a new cluster; no post-hoc merges | Miranda et al. 2018 (centroid, immutable assignments) |
| Threshold tau_join | Do not copy an absolute value. Start at 0.80 to 0.85 for bge-small on title+lede, then calibrate on SemEval "overall news story" pairs and the background pair distribution | BGE FAQ (distribution in [0.6, 1]); start value UNVERIFIED |
| Embedding input | Title + first ~100 to 150 words (lede), truncated to fit the model (256 word pieces for all-MiniLM-L6-v2) | all-MiniLM-L6-v2 card (verified); lede choice is a recommendation |
| Extra gate | Entity or date overlap required in addition to embedding similarity | Standard TDT practice; UNVERIFIED as measured gain |
| Shingle size | 5 words over boilerplate-stripped, normalized body | Lee et al. 2022 (5-grams); Henzinger 2006 (8 tokens) |
| Jaccard threshold | 0.8 on MinHash estimate, verified with an edit or containment check | Lee et al. 2022 (0.8 Jaccard, 0.8 edit similarity) |
| Novelty heuristic | Unique-fact ratio over normalized numbers, dates, capitalized spans, and quotes vs. earlier cluster members; use for ranking | TREC Novelty (redundancy as Boolean sentence-level judgment); heuristic is mine |
| Summary verification | Sentence-level NLI on claims vs. cited sources, plus a deterministic number/quote check | ALCE (citation recall/precision), SummaC (sentence-level NLI) |

## Sources (all URLs)

- https://arxiv.org/abs/1809.00540 (Miranda et al. 2018)
- https://aclanthology.org/D18-1483/
- https://preview.aclanthology.org/fix_video/N10-1021.pdf (Petrovic et al. 2010)
- https://arxiv.org/pdf/2508.08272 (Real-time News Story Identification)
- https://www.www2007.cpsc.ucalgary.ca/papers/paper570.pdf (Das et al. 2007)
- https://huggingface.co/BAAI/bge-large-en-v1.5 (BGE FAQ; same family as https://huggingface.co/BAAI/bge-small-en-v1.5)
- https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2
- https://sbert.net/docs/package_reference/util.html
- https://zenodo.org/records/6507872 (SemEval-2022 Task 8 data)
- https://aclanthology.org/2022.semeval-1.155/
- https://github.com/rupnikj/jair_paper
- https://github.com/priberam/news-clustering
- https://www.ldc.upenn.edu/sites/default/files/tdt2000-tdt3-corpus.pdf
- https://huggingface.co/datasets/alexfabbri/multi_news
- https://arxiv.org/pdf/2107.06499 (Lee et al. 2022)
- https://github.com/google-research/deduplicate-text-datasets
- https://infoscience.epfl.ch/record/99373/files/Henzinger06.pdf?version=1 (Henzinger 2006)
- https://arxiv.org/pdf/cs/0510054 (TREC Novelty track overview)
- https://pages.nist.gov/trec-browser/trec13/novelty/overview/
- https://arxiv.org/pdf/2305.14627 (ALCE)
- https://github.com/fallcat/ALCE
- https://arxiv.org/pdf/2111.09525 (SummaC)
