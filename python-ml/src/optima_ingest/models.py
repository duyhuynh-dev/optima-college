from dataclasses import dataclass


@dataclass(frozen=True)
class SubjectLink:
    term: str
    subject_code: str
    href: str


@dataclass(frozen=True)
class CourseRecord:
    term: str
    term_label: str
    subject_code: str
    course_code: str
    course_number: str
    course_title: str
    course_ref: str
    source_url: str


@dataclass(frozen=True)
class SectionRecord:
    term: str
    term_label: str
    subject_code: str
    course_code: str
    course_ref: str
    section: str
    instructor: str
    meeting_pattern: str
    source_url: str


@dataclass(frozen=True)
class MeetingRecord:
    term: str
    term_label: str
    subject_code: str
    course_code: str
    course_ref: str
    section: str
    day_code: str
    day_name: str
    start_time: str
    end_time: str
    source_url: str
