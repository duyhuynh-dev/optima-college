from optima_ingest.wesmaps import parse_subject_page


def test_parse_subject_page_extracts_section_fields() -> None:
    html = """
    <html><body>
      <tr><td class='header' colspan='3'>WesMaps - Fall 2026 Courses Offered</td></tr>
      <tr>
        <td valign='top' width='5%' nowrap>
          <a href='!wesmaps_page.html?stuid=&facid=NONE&crse=003328&term=1269'>COMP112-01</a>
        </td>
        <td valign='top' width='55%'>Introduction to Programming</td>
        <td valign='top' width='40%'>
          <a target='_blank' href='http://example.com'>Thayer,Kelly</a><br>
          ..T.... 08:50AM-10:10AM; ....R.. 08:50AM-10:10AM;
        </td>
      </tr>
    </body></html>
    """

    courses, sections, meetings = parse_subject_page(
        html,
        term="1269",
        subject_code="COMP",
        source_url="https://owaprod-pub.wesleyan.edu/reg/!wesmaps_page.html?crse_list=COMP&term=1269&offered=Y",
    )

    assert len(courses) == 1
    assert len(sections) == 1
    assert len(meetings) == 2

    course = courses[0]
    section = sections[0]

    assert course.term_label == "Fall 2026"
    assert course.course_code == "COMP112"
    assert course.course_number == "112"
    assert course.course_ref == "003328"
    assert course.course_title == "Introduction to Programming"
    assert course.prereq_groups == "[]"

    assert section.section == "01"
    assert section.instructor == "Thayer,Kelly"
    assert section.credits == "1.0"
    assert "08:50AM-10:10AM" in section.meeting_pattern
    assert {m.day_code for m in meetings} == {"T", "R"}
    assert all(m.start_time == "08:50AM" for m in meetings)
    assert all(m.end_time == "10:10AM" for m in meetings)
