"""Compatibility launcher for Boosty Downloader used by the desktop app."""

from __future__ import annotations

import contextvars
import re
from urllib.parse import parse_qs, urlencode, urlparse, urlunparse

from yt_dlp.utils import smuggle_url

from boosty_downloader.application.use_cases.download_single_post import (
    DownloadSinglePostUseCase,
)
from boosty_downloader.infrastructure.external_videos_downloader.external_videos_downloader import (
    ExternalVideosDownloader,
)


_boosty_referer: contextvars.ContextVar[str | None] = contextvars.ContextVar(
    "boosty_referer", default=None
)
_original_download_external_video = DownloadSinglePostUseCase.download_external_videos
_original_download_video = ExternalVideosDownloader.download_video


def _vimeo_player_url(url: str) -> str | None:
    parsed = urlparse(url)
    if (parsed.hostname or "").lower() not in {
        "vimeo.com",
        "www.vimeo.com",
        "player.vimeo.com",
    }:
        return None

    parts = [part for part in parsed.path.split("/") if part]
    video_index = next(
        (index for index, part in enumerate(parts) if re.fullmatch(r"\d+", part)),
        None,
    )
    if video_index is None:
        return None

    video_id = parts[video_index]
    query = parse_qs(parsed.query, keep_blank_values=True)
    if "h" not in query and len(parts) > video_index + 1:
        possible_hash = parts[video_index + 1]
        if re.fullmatch(r"[a-zA-Z0-9]+", possible_hash):
            query["h"] = [possible_hash]

    return urlunparse(
        ("https", "player.vimeo.com", f"/video/{video_id}", "", urlencode(query, doseq=True), "")
    )


async def _download_external_video_with_post_context(
    self: DownloadSinglePostUseCase, external_video: object
):
    post_url = (
        f"https://boosty.to/{self.context.author_name}/posts/{self.post_dto.id}"
    )
    token = _boosty_referer.set(post_url)
    try:
        return await _original_download_external_video(self, external_video)
    finally:
        _boosty_referer.reset(token)


def _download_video_with_vimeo_embed(
    self: ExternalVideosDownloader,
    url: str,
    destination_directory,
    progress_hook=None,
):
    player_url = _vimeo_player_url(url)
    referer = _boosty_referer.get()
    if player_url and referer:
        url = smuggle_url(player_url, {"referer": referer})
    return _original_download_video(
        self,
        url=url,
        destination_directory=destination_directory,
        progress_hook=progress_hook,
    )


DownloadSinglePostUseCase.download_external_videos = (
    _download_external_video_with_post_context
)
ExternalVideosDownloader.download_video = _download_video_with_vimeo_embed


if __name__ == "__main__":
    from boosty_downloader.main import entry_point

    entry_point()
