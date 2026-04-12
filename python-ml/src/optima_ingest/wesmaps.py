from __future__ import annotations

import csv
import re
from dataclasses import asdict
from pathlib import Path
from typing import Optional
from urllib.parse import parse_qs, urljoin, urlparse

import requests
from bs4 import BeautifulSoup

from .catalog_detail import enrich_from_details
from .models import CourseRecord, MeetingRecord, SectionRecord, SubjectLink

BASE_URL = "https://owaprod-pub.wesleyan.edu/reg/!wesmaps_page.html"
HEADER_TERM_RE = re.compile(r"(Fall|Spring|Summer|Winter)\s+\d{4}", re.IGNORECASE)
SECTION_CODE_RE = re.compile(r"^([A-Z]{2,5})(\d{3}[A-Z]?)-([0-9A-Z]{2,3})$")
MEETING_BLOCK_RE = re.compile(r"([\.MTWRFSU]{7})\s+(\d{1,2}:\d{2}[AP]M)-(\d{1,2}:\d{2}[AP]M)")
# WesMaps day masks are Sunday->Saturday (U M T W R F S), e.g. "..T....".
DAY_INDEX = [
    ("U", "Sunday"),
    ("M", "Monday"),
    ("T", "Tuesday"),
    ("W", "Wednesday"),
    ("R", "Thursday"),
    ("F", "Friday"),
    ("S", "Saturday"),
]


def fetch_html(url: str, timeout_seconds: int = 20) -> str:
    response = requests.get(url, timeout=timeout_seconds)
    response.raise_for_status()
    return response.text


def discover_subject_links(index_html: str, term: str) -> list[SubjectLink]:
    soup = BeautifulSoup(index_html, "html.parser")
    links: list[SubjectLink] = []

    for a_tag in soup.select("a[href]"):
        href = a_tag.get("href", "")
        if "subj_page=" not in href or "term=" not in href:
            continue

        full_url = urljoin(BASE_URL, href)
        parsed = urlparse(full_url)
        query = parse_qs(parsed.query)
        subject_code = (query.get("subj_page") or [""])[0].strip().upper()
        discovered_term = (query.get("term") or [""])[0].strip()

        if not subject_code or discovered_term != term:
            continue

        links.append(SubjectLink(term=term, subject_code=subject_code, href=full_url))

    unique = {(item.term, item.subject_code): item for item in links}
    return sorted(unique.values(), key=lambda item: item.subject_code)


def build_offered_courses_url(term: str, subject_code: str) -> str:
    return f"{BASE_URL}?stuid=&facid=NONE&crse_list={subject_code}&term={term}&offered=Y"


def normalize_meeting_pattern(text: str) -> str:
    return " ".join(text.split())


def parse_meetings(
    *,
    term: str,
    term_label: str,
    subject_code: str,
    course_code: str,
    course_ref: str,
    section: str,
    meeting_pattern: str,
    source_url: str,
) -> list[MeetingRecord]:
    meetings: list[MeetingRecord] = []
    for day_mask, start_time, end_time in MEETING_BLOCK_RE.findall(meeting_pattern):
        for idx, (day_code, day_name) in enumerate(DAY_INDEX):
            if idx >= len(day_mask) or day_mask[idx] == ".":
                continue
            meetings.append(
                MeetingRecord(
                    term=term,
                    term_label=term_label,
                    subject_code=subject_code,
                    course_code=course_code,
                    course_ref=course_ref,
                    section=section,
                    day_code=day_code,
                    day_name=day_name,
                    start_time=start_time,
                    end_time=end_time,
                    source_url=source_url,
                )
            )
    return meetings


def parse_row(
    row, *, term: str, term_label: str, subject_code: str, source_url: str
) -> tuple[Optional[CourseRecord], Optional[SectionRecord], list[MeetingRecord]]:
    cells = row.find_all("td")
    if len(cells) < 3:
        return None, None, []

    section_anchor = cells[0].find("a")
    if not section_anchor:
        return None, None, []

    section_code = section_anchor.get_text(strip=True).upper()
    section_match = SECTION_CODE_RE.match(section_code)
    if not section_match:
        return None, None, []

    parsed_subject_code = section_match.group(1)
    if parsed_subject_code != subject_code:
        return None, None, []

    course_number = section_match.group(2)
    section = section_match.group(3)
    course_code = f"{parsed_subject_code}{course_number}"

    section_href = section_anchor.get("href", "")
    parsed_href = parse_qs(urlparse(urljoin(BASE_URL, section_href)).query)
    course_ref = (parsed_href.get("crse") or [""])[0].strip()

    course_title = " ".join(cells[1].get_text(" ", strip=True).split())
    instructor_anchor = cells[2].find("a")
    instructor = (
        " ".join(instructor_anchor.get_text(" ", strip=True).split())
        if instructor_anchor
        else "TBD"
    )
    meeting_pattern = normalize_meeting_pattern(cells[2].get_text(" ", strip=True))

    course = CourseRecord(
        term=term,
        term_label=term_label,
        subject_code=subject_code,
        course_code=course_code,
        course_number=course_number,
        course_title=course_title,
        course_ref=course_ref,
        source_url=source_url,
        prereq_groups="[]",
    )
    section_record = SectionRecord(
        term=term,
        term_label=term_label,
        subject_code=subject_code,
        course_code=course_code,
        course_ref=course_ref,
        section=section,
        instructor=instructor,
        meeting_pattern=meeting_pattern,
        source_url=source_url,
        credits="1.0",
    )
    meetings = parse_meetings(
        term=term,
        term_label=term_label,
        subject_code=subject_code,
        course_code=course_code,
        course_ref=course_ref,
        section=section,
        meeting_pattern=meeting_pattern,
        source_url=source_url,
    )
    return course, section_record, meetings


def parse_subject_page(
    html: str, *, term: str, subject_code: str, source_url: str
) -> tuple[list[CourseRecord], list[SectionRecord], list[MeetingRecord]]:
    soup = BeautifulSoup(html, "html.parser")

    courses: list[CourseRecord] = []
    sections: list[SectionRecord] = []
    meetings: list[MeetingRecord] = []
    active_term_label = ""

    for row in soup.find_all("tr"):
        header_cell = row.find("td", class_="header")
        if header_cell:
            header_text = " ".join(header_cell.get_text(" ", strip=True).split())
            term_match = HEADER_TERM_RE.search(header_text)
            if term_match:
                active_term_label = term_match.group(0).title()
            continue

        course, section_record, meeting_rows = parse_row(
            row,
            term=term,
            term_label=active_term_label,
            subject_code=subject_code,
            source_url=source_url,
        )
        if course:
            courses.append(course)
        if section_record:
            sections.append(section_record)
        meetings.extend(meeting_rows)

    unique_courses = {
        (c.term, c.term_label, c.subject_code, c.course_code, c.course_ref): c for c in courses
    }
    unique_sections = {
        (s.term, s.term_label, s.subject_code, s.course_code, s.section, s.course_ref): s
        for s in sections
    }
    unique_meetings = {
        (
            m.term,
            m.term_label,
            m.subject_code,
            m.course_code,
            m.section,
            m.day_code,
            m.start_time,
            m.end_time,
            m.course_ref,
        ): m
        for m in meetings
    }
    return list(unique_courses.values()), list(unique_sections.values()), list(unique_meetings.values())


def write_bronze_snapshot(path: Path, html: str) -> None:
    """Persist raw HTML (bronze layer) for replay and parser versioning."""
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(html, encoding="utf-8")


def write_csv(path: Path, rows: list[dict[str, str]]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    if not rows:
        path.write_text("", encoding="utf-8")
        return

    fieldnames = list(rows[0].keys())
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)


def run_ingestion(
    term: str,
    out_dir: Path,
    *,
    save_bronze: bool = True,
    enrich: bool = False,
    enrich_workers: int = 8,
) -> None:
    bronze_root = out_dir / "bronze" / term
    index_url = f"{BASE_URL}?stuid=&facid=NONE&term={term}"
    index_html = fetch_html(index_url)
    if save_bronze:
        write_bronze_snapshot(bronze_root / "index.html", index_html)
    subject_links = discover_subject_links(index_html, term=term)

    all_courses: list[CourseRecord] = []
    all_sections: list[SectionRecord] = []
    all_meetings: list[MeetingRecord] = []

    for subject in subject_links:
        offered_url = build_offered_courses_url(term=term, subject_code=subject.subject_code)
        subject_html = fetch_html(offered_url)
        if save_bronze:
            safe_code = subject.subject_code.replace("/", "_")
            write_bronze_snapshot(bronze_root / f"subj_{safe_code}.html", subject_html)
        courses, sections, meetings = parse_subject_page(
            subject_html,
            term=term,
            subject_code=subject.subject_code,
            source_url=offered_url,
        )
        all_courses.extend(courses)
        all_sections.extend(sections)
        all_meetings.extend(meetings)

    if enrich:
        all_courses, all_sections = enrich_from_details(
            term, all_courses, all_sections, max_workers=enrich_workers
        )

    course_rows = [asdict(course) for course in all_courses]
    section_rows = [asdict(section) for section in all_sections]
    meeting_rows = [asdict(meeting) for meeting in all_meetings]

    write_csv(out_dir / f"courses_{term}.csv", course_rows)
    write_csv(out_dir / f"sections_{term}.csv", section_rows)
    write_csv(out_dir / f"meetings_{term}.csv", meeting_rows)
