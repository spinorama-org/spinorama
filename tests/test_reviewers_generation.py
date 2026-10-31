from scripts.update_reviewers import reviewer_options
from src.website.utils import get_reviews


def test_reviewers_include_new_origins_and_misc_versions():
    metadata = {
        "speaker": {"measurements": {
            "speakerlab": {"origin": "SpeakerLab"},
            "misc-audioxpress-horizontal": {"origin": "Misc"},
            "other": {"origin": "SpeakerLab"},
            "vendor": {"origin": "Vendors-A&B"},
        }}
    }
    options = list(reviewer_options(metadata))
    assert '<option value="SpeakerLab">SpeakerLab</option>' in options
    assert '<option value="audioxpress">Audio Xpress</option>' in options
    assert '<option value="Vendors-A&amp;B">Vendors-A&amp;B</option>' in options
    assert len(options) == 3


def test_python_review_link_for_speakerlab():
    speaker = {
        "brand": "Philips",
        "model": "Fidelio SoundSphere DS9800W",
        "measurements": {"speakerlab": {"origin": "SpeakerLab"}},
    }
    review = get_reviews({}, speaker)[0]
    assert review["originLong"] == "SpeakerLab"
    assert review["url"] == (
        "speakers/Philips%20Fidelio%20SoundSphere%20DS9800W/"
        "SpeakerLab/index_speakerlab.html"
    )
