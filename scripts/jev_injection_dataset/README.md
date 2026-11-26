# Jev injection back-test dataset

Rebuilds the dataset for `jev_backtest injection` and `injection-order` (064 T038 (e), 065 T028a). Run from a fresh, empty folder outside the repository, with Python's `-I` flag: `python -I fetch_attacks.py OUT_DIR`, then `python -I fetch_clean.py OUT_DIR` (needs `EXA_API_KEY`, about $0.62 in Exa calls), then `python -I build.py OUT_DIR`; the dataset is `OUT_DIR/dataset/`.
