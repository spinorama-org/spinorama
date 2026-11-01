#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Summarise nginx access logs to separate humans, crawlers, and scrapers.

Pure standard library, meant to run on the build machine against log
files copied from the web host (supports .gz). It only reads logs.

Example:
    .venv/bin/python scripts/analyze_access_log.py access.log access.log.1.gz
"""

from __future__ import annotations

import argparse
import gzip
import re
import sys
from collections import Counter
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlparse

BOT_UA_PATTERNS = (
    "curl", "wget", "python-requests", "python-httpx", "httpx", "aiohttp",
    "scrapy", "go-http-client", "java/", "libwww", "okhttp", "node-fetch",
    "axios", "headlesschrome", "phantomjs", "selenium", "puppeteer",
    "playwright", "mechanize", "httpclient", "masscan", "zgrab", "nmap",
    "gptbot", "claudebot", "ccbot", "amazonbot", "bytespider", "petalbot",
    "semrushbot", "ahrefsbot", "mj12bot", "dotbot", "dataforseobot",
    "serpstatbot", "blexbot", "zoominfo", "sogou", "yandexbot", "baiduspider",
    "palo alto networks", "censys", "shodan", "internetmeasurement",
)
PROBE_PATTERNS = (
    "wp-", "xmlrpc", ".php", ".env", "cgi-bin", "sdk/weblanguage",
    ".git/", ".aws/", "phpmyadmin", "vendor/phpunit", "boaform",
    "actuator/", ".ds_store", "config.json.bak",
)
HONEYPOT_PATHS = (
    "/trap-7f3a.html",
    "/speakers/__honeypot__/cea2034.json",
    "/headphones/__honeypot__/frequency_response.csv",
)

_LINE = re.compile(
    r'^(?P<ip>\S+) \S+ \S+ \[[^\]]+\] "(?P<method>\S+) (?P<path>\S+) [^"]*" '
    r'(?P<status>\d{3}) (?P<size>\S+) "[^"]*" "(?P<ua>[^"]*)"'
)


@dataclass(frozen=True)
class Record:
    ip: str
    method: str
    path: str
    status: int
    ua: str


def parse_line(line: str) -> Record | None:
    match = _LINE.match(line)
    if match is None:
        return None
    return Record(
        ip=match["ip"],
        method=match["method"],
        path=match["path"],
        status=int(match["status"]),
        ua=match["ua"],
    )


def is_bot_ua(ua: str) -> bool:
    if not ua or ua == "-":
        return True
    lower = ua.lower()
    return any(pattern in lower for pattern in BOT_UA_PATTERNS)


def is_probe(path: str) -> bool:
    lower = unquote(path).lower()
    return any(pattern in lower for pattern in PROBE_PATTERNS)


def is_honeypot(path: str) -> bool:
    return urlparse(path).path.lower() in HONEYPOT_PATHS


def speaker_of(path: str) -> str | None:
    parts = [unquote(p) for p in urlparse(path).path.split("/") if p]
    if len(parts) >= 2 and parts[0] in ("speakers", "headphones"):
        return parts[1]
    return None


def classify(record: Record) -> str:
    path = urlparse(record.path).path
    lower = path.lower()
    if is_honeypot(path):
        return "honeypot"
    if is_probe(path):
        return "probe"
    if lower.startswith("/compare-render/"):
        return "compare_render"
    if lower.startswith("/json/"):
        return "metadata_json"
    if lower.startswith("/speakers/") or lower.startswith("/headphones/"):
        if lower.endswith(".json"):
            return "speaker_json"
        if lower.endswith((".webp", ".jpg", ".jpeg", ".png")):
            return "graph_image"
        if lower.endswith(".csv"):
            return "speaker_csv"
        return "speaker_page"
    if lower.endswith((".js", ".css")):
        return "static_asset"
    if lower.endswith((".html", "/")) or "." not in lower.rsplit("/", 1)[-1]:
        return "html"
    return "other"


def summarize(records: list[Record]) -> dict:
    by_class: Counter[str] = Counter()
    by_ip: Counter[str] = Counter()
    by_ua: Counter[str] = Counter()
    statuses: Counter[int] = Counter()
    speakers_by_ip: dict[str, set[str]] = {}
    bot_requests = 0
    for record in records:
        by_class[classify(record)] += 1
        by_ip[record.ip] += 1
        by_ua[record.ua] += 1
        statuses[record.status] += 1
        if is_bot_ua(record.ua):
            bot_requests += 1
        speaker = speaker_of(record.path)
        if speaker:
            speakers_by_ip.setdefault(record.ip, set()).add(speaker)
    return {
        "total": len(records),
        "bot_ua_requests": bot_requests,
        "by_class": dict(by_class),
        "by_status": dict(statuses),
        "top_ips": by_ip.most_common(20),
        "top_uas": by_ua.most_common(20),
        "unique_speakers": {ip: len(names) for ip, names in speakers_by_ip.items()},
        "top_speaker_harvesters": sorted(
            ((len(names), ip) for ip, names in speakers_by_ip.items()), reverse=True
        )[:20],
    }


def read_records(paths: list[str]) -> list[Record]:
    records: list[Record] = []
    for name in paths:
        opener = gzip.open if name.endswith(".gz") else open
        with opener(name, "rt", encoding="utf-8", errors="replace") as handle:
            for line in handle:
                record = parse_line(line)
                if record is not None:
                    records.append(record)
    return records


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("logs", nargs="+")
    args = parser.parse_args(argv)
    records = read_records(args.logs)
    summary = summarize(records)
    total = summary["total"] or 1
    print(f"total requests: {summary['total']}")
    print(
        f"bot-UA requests: {summary['bot_ua_requests']} "
        f"({100 * summary['bot_ua_requests'] / total:.1f}%)"
    )
    print("\nby class:")
    for name, count in sorted(summary["by_class"].items(), key=lambda kv: -kv[1]):
        print(f"  {name:20s} {count:8d}  {100 * count / total:5.1f}%")
    print("\nby status:")
    for status, count in sorted(summary["by_status"].items()):
        print(f"  {status}  {count}")
    print("\ntop IPs:")
    for ip, count in summary["top_ips"]:
        print(f"  {ip:40s} {count}")
    print("\ntop speaker harvesters (unique speakers, IP):")
    for count, ip in summary["top_speaker_harvesters"]:
        print(f"  {ip:40s} {count}")
    print("\ntop user agents:")
    for ua, count in summary["top_uas"]:
        print(f"  {count:8d}  {ua[:100]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
