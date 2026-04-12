"""
Fetch per-course WesMaps detail pages for credit hours and prerequisite text.

Used by optional ingest enrichment (--enrich). Public listing pages do not include
these fields reliably; detail URLs use ?crse=<course_ref>&term=<term>.
"""

from __future__ import annotations

import json
import re
from concurrent.futures import ThreadPoolExecutor, as_completed

import requests
from bs4 import BeautifulSoup

from .models import CourseRecord, SectionRecord

BASE_URL = "https://owaprod-pub.wesleyan.edu/reg/!wesmaps_page.html"
# Catalog-style codes appearing in prose (subject + 3 digits + optional letter).
PREREQ_CODE_RE = re.compile(r"\b([A-Z]{2,5}\d{3}[A-Z]?)\b", re.IGNORECASE)
CREDIT_RE = re.compile(r"Credit:\s*([\d.]+)", re.IGNORECASE)


def parse_prerequisite_groups(text: str) -> list[list[str]]:
    """
    Turn WesMaps prerequisite prose into AND-of-OR groups.
    Each inner list is alternatives (need at least one); outer list is AND across groups.
    """
    if not text:
        return []
    raw = re.sub(r"^Prerequisites?:\s*", "", text, flags=re.I).strip()
    if not raw or raw.lower() == "none":
        return []
    and_parts = re.split(r"\s+AND\s+", raw, flags=re.I)
    groups: list[list[str]] = []
    for part in and_parts:
        part = part.strip().strip("()")
        if not part:
            continue
        or_parts = re.split(r"\s+OR\s+", part, flags=re.I)
        codes: list[str] = []
        for frag in or_parts:
            for m in PREREQ_CODE_RE.finditer(frag):
                c = m.group(1).upper()
                if c not in codes:
                    codes.append(c)
        if codes:
            groups.append(codes)
    return groups


def _parse_detail_html(html: str) -> tuple[float | None, list[list[str]]]:
    soup = BeautifulSoup(html, "html.parser")
    credit_val: float | None = None
    prereq_text: str | None = None
    for tr in soup.find_all("tr"):
        for td in tr.find_all("td"):
            text = td.get_text(" ", strip=True)
            if credit_val is None:
                m = CREDIT_RE.search(text)
                if m:
                    try:
                        credit_val = float(m.group(1))
                    except ValueError:
                        pass
            if prereq_text is None and text.upper().startswith("PREREQUISITE"):
                prereq_text = text
    groups = parse_prerequisite_groups(prereq_text or "")
    return credit_val, groups


def fetch_course_detail(term: str, course_ref: str, *, timeout: float = 25.0) -> tuple[float | None, list[list[str]]]:
    if not course_ref.strip():
        return None, []
    url = f"{BASE_URL}?stuid=&facid=NONE&crse={course_ref.strip()}&term={term.strip()}"
    r = requests.get(url, timeout=timeout)
    r.raise_for_status()
    return _parse_detail_html(r.text)


def _format_credit(c: float) -> str:
    if abs(c - round(c)) < 1e-9:
        return str(int(round(c)))
    return f"{c:g}"


def enrich_from_details(
    term: str,
    courses: list[CourseRecord],
    sections: list[SectionRecord],
    *,
    max_workers: int = 8,
) -> tuple[list[CourseRecord], list[SectionRecord]]:
    """
    Mutate-free: returns new course/section lists with credits + prereq_groups filled from detail pages.
    """
    refs = sorted({c.course_ref.strip() for c in courses if c.course_ref.strip()})
    if not refs:
        return courses, sections

    ref_detail: dict[str, tuple[float | None, list[list[str]]]] = {}

    def job(ref: str) -> tuple[str, tuple[float | None, list[list[str]]]]:
        try:
            return ref, fetch_course_detail(term, ref)
        except Exception:
            return ref, (None, [])

    with ThreadPoolExecutor(max_workers=max_workers) as pool:
        futures = [pool.submit(job, ref) for ref in refs]
        for fut in as_completed(futures):
            ref, payload = fut.result()
            ref_detail[ref] = payload

    ref_to_groups: dict[str, str] = {}
    ref_to_credit: dict[str, str] = {}
    for ref, (cr, groups) in ref_detail.items():
        ref_to_groups[ref] = json.dumps(groups, separators=(",", ":"))
        if cr is not None:
            ref_to_credit[ref] = _format_credit(cr)

    new_courses: list[CourseRecord] = []
    for c in courses:
        g = ref_to_groups.get(c.course_ref.strip(), "[]")
        new_courses.append(
            CourseRecord(
                term=c.term,
                term_label=c.term_label,
                subject_code=c.subject_code,
                course_code=c.course_code,
                course_number=c.course_number,
                course_title=c.course_title,
                course_ref=c.course_ref,
                source_url=c.source_url,
                prereq_groups=g,
            )
        )

    new_sections: list[SectionRecord] = []
    for s in sections:
        cr_str = ref_to_credit.get(s.course_ref.strip(), s.credits)
        new_sections.append(
            SectionRecord(
                term=s.term,
                term_label=s.term_label,
                subject_code=s.subject_code,
                course_code=s.course_code,
                course_ref=s.course_ref,
                section=s.section,
                instructor=s.instructor,
                meeting_pattern=s.meeting_pattern,
                source_url=s.source_url,
                credits=cr_str,
            )
        )

    return new_courses, new_sections
