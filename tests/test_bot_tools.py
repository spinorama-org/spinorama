"""Access-log analysis and scraper-ban candidate generation."""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "scripts"))

import analyze_access_log
import ban_scrapers


def _line(ip, path, ua="Mozilla/5.0", status=200, method="GET"):
    return f'{ip} - - [31/Oct/2026:10:00:00 +0100] "{method} {path} HTTP/1.1" {status} 1234 "-" "{ua}"'


SAMPLE = [
    _line("1.2.3.4", "/index.html"),
    _line("1.2.3.4", "/speakers/Foo/ASR/asr/CEA2034.json", ua="python-requests/2.31"),
    _line("1.2.3.4", "/speakers/Bar/ASR/asr/CEA2034.json", ua="python-requests/2.31"),
    _line("5.6.7.8", "/trap-7f3a.html", ua="curl/8.0", status=444),
    _line("9.9.9.9", "/wp-login.php", status=404),
    _line("9.9.9.9", "/.env", status=404),
    _line("9.9.9.9", "/xmlrpc.php", method="POST", status=404),
    _line("2.2.2.2", "/speakers/Foo/ASR/asr/CEA2034.webp"),
]


def test_parse_and_classify():
    records = [analyze_access_log.parse_line(line) for line in SAMPLE]
    assert all(r is not None for r in records)
    summary = analyze_access_log.summarize(records)
    assert summary["total"] == 8
    assert summary["by_class"]["speaker_json"] == 2
    assert summary["by_class"]["probe"] == 3
    assert summary["unique_speakers"]["1.2.3.4"] == 2
    assert analyze_access_log.is_bot_ua("python-requests/2.31")
    assert analyze_access_log.is_bot_ua("curl/8.0")
    assert not analyze_access_log.is_bot_ua("Mozilla/5.0")
    assert analyze_access_log.is_honeypot("/trap-7f3a.html")
    assert analyze_access_log.is_probe("/wp-login.php")


def test_ban_candidates():
    records = [ban_scrapers.parse_line(line) for line in SAMPLE]
    thresholds = ban_scrapers.Thresholds(
        honeypot_hits=1, probe_hits=3, speaker_json=2, unique_speakers=50, total_requests=1000
    )
    candidates = ban_scrapers.find_candidates(records, thresholds, allowlist=set())
    ips = {c.ip for c in candidates}
    assert "5.6.7.8" in ips  # honeypot
    assert "9.9.9.9" in ips  # probes
    assert "1.2.3.4" in ips  # speaker JSON harvesting
    assert "2.2.2.2" not in ips
    # allowlisted IPs are never candidates
    candidates = ban_scrapers.find_candidates(records, thresholds, allowlist={"5.6.7.8"})
    assert "5.6.7.8" not in {c.ip for c in candidates}
