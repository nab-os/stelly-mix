// The transport. Owns the one <audio> element, so seeking and progress never
// wait on Rust; tells Rust where it is, a few times a second.
//
// A click on a waveform (`.wave[data-url]`) plays its file from where it
// landed; Rust learns of it from the same click, and of the position from
// the time updates.

(async () => {
  let audio = document.getElementById("player");
  for (let attempt = 0; attempt < 100 && !audio; attempt++) {
    await new Promise((resolve) => setTimeout(resolve, 50));
    audio = document.getElementById("player");
  }
  if (!audio) return;

  const report = () =>
    dioxus.send({
      type: "time",
      url: audio.currentSrc || "",
      position: audio.currentTime || 0,
      duration: Number.isFinite(audio.duration) ? audio.duration : 0,
      playing: !audio.paused,
    });

  window.mixPlay = (url, start) => {
    if (audio.currentSrc !== url) audio.src = url;
    const go = () => {
      audio.currentTime = Math.max(0, start || 0);
      audio.play().catch(() => {});
    };
    if (audio.readyState >= 1) go();
    else audio.addEventListener("loadedmetadata", go, { once: true });
  };
  window.mixToggle = () => (audio.paused ? audio.play().catch(() => {}) : audio.pause());
  window.mixSeek = (seconds) => {
    audio.currentTime = Math.max(0, seconds);
  };

  // A waveform says what it plays and how long that is; where the click
  // lands is the time to play from.
  document.addEventListener("click", (event) => {
    const wave = event.target.closest && event.target.closest(".wave[data-url]");
    if (!wave) return;
    const box = wave.getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (event.clientX - box.left) / box.width));
    window.mixPlay(wave.dataset.url, fraction * Number(wave.dataset.seconds || 0));
  });

  // Space plays and pauses, unless someone is typing.
  document.addEventListener("keydown", (event) => {
    const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(event.target.tagName);
    if (event.code === "Space" && !typing && audio.currentSrc) {
      event.preventDefault();
      window.mixToggle();
    }
  });

  for (const name of ["timeupdate", "play", "pause", "ended", "loadedmetadata", "seeked"]) {
    audio.addEventListener(name, report);
  }
  audio.addEventListener("error", () =>
    dioxus.send({ type: "error", url: audio.currentSrc || "", message: audio.error ? audio.error.message || String(audio.error.code) : "" }),
  );
})();
