(function () {
  const page = document.body?.dataset?.page;
  document.querySelectorAll(".nav a").forEach((a) => {
    const href = a.getAttribute("href") || "";
    if ((page === "home" && href.includes("index.html")) || href.includes(`${page}.html`)) {
      a.classList.add("is-active");
    }
  });
})();
