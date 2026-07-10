(() => {
  "use strict";

  const $ = (selector, scope = document) => scope.querySelector(selector);
  const elements = {
    form: $("#job-form"),
    alert: $("#global-alert"),
    search: $("#school-search"),
    searchResults: $("#school-results"),
    searchStatus: $("#school-search-status"),
    selectedSchools: $("#selected-schools"),
    schoolCount: $("#school-count"),
    clearSchools: $("#clear-schools"),
    allSchools: $("#all-active-schools"),
    allSchoolCount: $("#all-school-count"),
    yearOptions: $("#year-options"),
    yearMessage: $("#year-message"),
    estimate: $("#request-estimate"),
    estimateDetail: $("#request-estimate-detail"),
    submit: $("#start-job"),
    concurrency: $("#setting-concurrency"),
    rate: $("#setting-rate"),
    timeout: $("#setting-timeout"),
    jobPanel: $("#job-panel"),
    jobTitle: $("#job-title"),
    jobCopy: $("#job-status-copy"),
    jobBadge: $("#job-state-badge"),
    progressBar: $("#progress-bar"),
    progressCompleted: $("#progress-completed"),
    progressTotal: $("#progress-total"),
    progressPercent: $("#progress-percent"),
    jobFailure: $("#job-failure"),
    quality: $("#quality-summary"),
    qualityUsable: $("#quality-usable"),
    qualitySuppressed: $("#quality-suppressed"),
    qualityMissing: $("#quality-missing"),
    qualityFailed: $("#quality-failed"),
    downloadArea: $("#download-area"),
    resultsSection: $("#results-section"),
    resultsHead: $("#results-head"),
    resultsBody: $("#results-body"),
    resultCount: $("#result-count"),
    tableFilter: $("#table-filter"),
  };

  const state = {
    schools: new Map(),
    years: new Set(),
    searchResults: [],
    activeResult: -1,
    searchTimer: null,
    searchController: null,
    polling: null,
    jobId: null,
    rows: [],
    totalSchoolCount: 0,
    allSchools: false,
  };

  const schoolFields = {
    cds: (school) => String(school.cds_code ?? school.cds ?? ""),
    name: (school) => school.school_name ?? school.school ?? school.name ?? "Unnamed school",
    district: (school) => school.district_name ?? school.district ?? "District unavailable",
    county: (school) => school.county_name ?? school.county ?? "County unavailable",
  };

  async function fetchJson(url, options = {}) {
    const response = await fetch(url, {
      headers: { Accept: "application/json", ...(options.body ? { "Content-Type": "application/json" } : {}) },
      ...options,
    });
    let data = null;
    const isJson = (response.headers.get("content-type") || "").includes("application/json");
    if (isJson) data = await response.json();
    if (!response.ok) {
      const message = data?.error || data?.message || `${response.status} ${response.statusText}`;
      throw new Error(message);
    }
    return data;
  }

  function showGlobalError(message) {
    elements.alert.textContent = message;
    elements.alert.hidden = false;
    elements.alert.scrollIntoView({ behavior: "smooth", block: "center" });
  }

  function clearGlobalError() {
    elements.alert.hidden = true;
    elements.alert.textContent = "";
  }

  function numberWithin(input, minimum, maximum) {
    const value = Number(input.value);
    return Number.isFinite(value) ? Math.max(minimum, Math.min(maximum, Math.round(value))) : minimum;
  }

  async function loadYears() {
    try {
      const response = await fetchJson("/api/years");
      const years = Array.isArray(response) ? response : response.years || [];
      elements.yearOptions.replaceChildren();
      years.forEach((entry, index) => {
        const year = Number(typeof entry === "object" ? entry.year : entry);
        if (!Number.isInteger(year)) return;
        const wrapper = document.createElement("div");
        wrapper.className = "year-option";
        const input = document.createElement("input");
        input.type = "checkbox";
        input.name = "years";
        input.id = `year-${year}`;
        input.value = String(year);
        input.checked = index >= Math.max(0, years.length - 3);
        if (input.checked) state.years.add(year);
        const label = document.createElement("label");
        label.htmlFor = input.id;
        label.textContent = String(year);
        input.addEventListener("change", () => {
          if (input.checked) state.years.add(year);
          else state.years.delete(year);
          elements.yearMessage.textContent = state.years.size ? "" : "Choose at least one reporting year.";
          updateEstimate();
        });
        wrapper.append(input, label);
        elements.yearOptions.append(wrapper);
      });
      if (!years.length) throw new Error("No supported reporting years were returned.");
      updateEstimate();
    } catch (error) {
      elements.yearOptions.textContent = "Reporting years could not be loaded.";
      showGlobalError(`Could not initialize the application: ${error.message}`);
    }
  }

  async function loadHealth() {
    try {
      const health = await fetchJson("/health");
      state.totalSchoolCount = Number(health.school_count) || 0;
      elements.allSchoolCount.textContent = state.totalSchoolCount
        ? state.totalSchoolCount.toLocaleString()
        : "available";
      updateEstimate();
    } catch (error) {
      showGlobalError(`Could not read the local school directory: ${error.message}`);
    }
  }

  function schoolMeta(school) {
    return `${schoolFields.district(school)} · ${schoolFields.county(school)} County · CDS ${schoolFields.cds(school)}`;
  }

  function renderSearchResults(results) {
    state.searchResults = results;
    state.activeResult = -1;
    elements.searchResults.replaceChildren();
    if (!results.length) {
      const empty = document.createElement("p");
      empty.className = "search-empty";
      empty.textContent = "No active schools matched that search.";
      elements.searchResults.append(empty);
    } else {
      results.forEach((school, index) => {
        const cds = schoolFields.cds(school);
        const button = document.createElement("button");
        button.type = "button";
        button.className = "search-result";
        button.id = `school-option-${index}`;
        button.setAttribute("role", "option");
        button.setAttribute("aria-selected", String(state.schools.has(cds)));
        button.tabIndex = -1;

        const check = document.createElement("span");
        check.className = "result-check";
        check.setAttribute("aria-hidden", "true");
        check.textContent = state.schools.has(cds) ? "✓" : "";
        const content = document.createElement("span");
        content.className = "search-result-content";
        const name = document.createElement("span");
        name.className = "search-result-name";
        name.textContent = schoolFields.name(school);
        const meta = document.createElement("span");
        meta.className = "search-result-meta";
        meta.textContent = schoolMeta(school);
        content.append(name, meta);
        button.append(check, content);
        button.addEventListener("mousedown", (event) => event.preventDefault());
        button.addEventListener("click", () => toggleSchool(school));
        elements.searchResults.append(button);
      });
    }
    elements.searchResults.hidden = false;
    elements.search.setAttribute("aria-expanded", "true");
    elements.searchStatus.textContent = `${results.length} matching ${results.length === 1 ? "school" : "schools"}.`;
  }

  function closeSearchResults() {
    elements.searchResults.hidden = true;
    elements.search.setAttribute("aria-expanded", "false");
    elements.search.removeAttribute("aria-activedescendant");
    state.activeResult = -1;
  }

  function setActiveResult(index) {
    const options = [...elements.searchResults.querySelectorAll(".search-result")];
    if (!options.length) return;
    state.activeResult = (index + options.length) % options.length;
    options.forEach((option, i) => option.classList.toggle("is-active", i === state.activeResult));
    const active = options[state.activeResult];
    elements.search.setAttribute("aria-activedescendant", active.id);
    active.scrollIntoView({ block: "nearest" });
  }

  async function searchSchools(query) {
    if (state.searchController) state.searchController.abort();
    state.searchController = new AbortController();
    elements.searchStatus.textContent = "Searching schools…";
    try {
      const response = await fetchJson(`/api/schools?q=${encodeURIComponent(query)}&limit=40`, {
        signal: state.searchController.signal,
      });
      renderSearchResults(Array.isArray(response) ? response : response.schools || []);
    } catch (error) {
      if (error.name === "AbortError") return;
      elements.searchStatus.textContent = "School search failed.";
      showGlobalError(`School search failed: ${error.message}`);
    }
  }

  function scheduleSearch() {
    if (state.allSchools) return;
    clearTimeout(state.searchTimer);
    const query = elements.search.value.trim();
    if (query.length < 2) {
      closeSearchResults();
      elements.searchStatus.textContent = query ? "Enter at least two characters." : "";
      return;
    }
    state.searchTimer = setTimeout(() => searchSchools(query), 180);
  }

  function toggleSchool(school) {
    if (state.allSchools) return;
    const cds = schoolFields.cds(school);
    if (!cds) return;
    if (state.schools.has(cds)) state.schools.delete(cds);
    else state.schools.set(cds, school);
    renderSelectedSchools();
    renderSearchResults(state.searchResults);
    elements.search.focus();
  }

  function renderSelectedSchools() {
    elements.selectedSchools.replaceChildren();
    const selectedCount = state.allSchools ? state.totalSchoolCount : state.schools.size;
    elements.schoolCount.textContent = selectedCount.toLocaleString();
    elements.clearSchools.hidden = state.allSchools || state.schools.size === 0;
    elements.selectedSchools.classList.toggle("empty-selection", state.allSchools || state.schools.size === 0);
    if (state.allSchools) {
      const all = document.createElement("p");
      all.textContent = `All ${state.totalSchoolCount.toLocaleString()} active schools will be included.`;
      elements.selectedSchools.append(all);
    } else if (!state.schools.size) {
      const empty = document.createElement("p");
      empty.textContent = "Search above to add schools.";
      elements.selectedSchools.append(empty);
    } else {
      state.schools.forEach((school, cds) => {
        const chip = document.createElement("span");
        chip.className = "school-chip";
        chip.title = schoolMeta(school);
        const label = document.createElement("span");
        label.textContent = `${schoolFields.name(school)} · ${cds}`;
        const remove = document.createElement("button");
        remove.type = "button";
        remove.textContent = "×";
        remove.setAttribute("aria-label", `Remove ${schoolFields.name(school)}, CDS ${cds}`);
        remove.addEventListener("click", () => {
          state.schools.delete(cds);
          renderSelectedSchools();
          if (!elements.searchResults.hidden) renderSearchResults(state.searchResults);
        });
        chip.append(label, remove);
        elements.selectedSchools.append(chip);
      });
    }
    updateEstimate();
  }

  function updateEstimate() {
    const schoolCount = state.allSchools ? state.totalSchoolCount : state.schools.size;
    const total = schoolCount * state.years.size;
    elements.estimate.textContent = total.toLocaleString();
    elements.estimateDetail.textContent = total
      ? `${schoolCount.toLocaleString()} ${schoolCount === 1 ? "school" : "schools"} × ${state.years.size} ${state.years.size === 1 ? "year" : "years"}`
      : "Choose schools and years";
    elements.submit.disabled = total === 0 || Boolean(state.polling);
  }

  function resetJobDisplay() {
    elements.jobFailure.hidden = true;
    elements.jobFailure.textContent = "";
    elements.quality.hidden = true;
    elements.downloadArea.hidden = true;
    elements.resultsSection.hidden = true;
    elements.progressBar.style.width = "0%";
  }

  function updateProgress(job) {
    const progress = job.progress || {};
    const completed = Number(progress.completed ?? job.completed ?? 0);
    const schoolCount = state.allSchools ? state.totalSchoolCount : state.schools.size;
    const total = Number(progress.total ?? job.total ?? schoolCount * state.years.size);
    const percent = total ? Math.min(100, Math.round((completed / total) * 100)) : 0;
    const status = String(job.status || "running").toLowerCase();
    elements.progressCompleted.textContent = completed.toLocaleString();
    elements.progressTotal.textContent = total.toLocaleString();
    elements.progressPercent.textContent = `${percent}%`;
    elements.progressBar.style.width = `${percent}%`;
    elements.jobBadge.textContent = status;
    elements.jobBadge.className = `state-badge ${status}`;
    if (status === "queued") {
      elements.jobTitle.textContent = "Preparing your report";
      elements.jobCopy.textContent = "The pull is queued and will begin shortly.";
    } else if (status === "running") {
      elements.jobTitle.textContent = "Pulling Dashboard data";
      elements.jobCopy.textContent = `Completed ${completed.toLocaleString()} of ${total.toLocaleString()} requests. You can keep this page open while the report runs.`;
    }
  }

  function readQuality(job) {
    const q = job.quality || job.quality_summary || {};
    const progress = job.progress || {};
    const sum = (...values) => values.reduce((total, value) => total + (Number(value) || 0), 0);
    // Prefer the exact DataQuality field names emitted by the Rust backend, with
    // legacy aliases kept as fallbacks. Suppressed combines privacy and
    // small-denominator suppression so no removed value is silently dropped.
    const usable = q.rows_reported ?? q.usable_rows ?? q.observed_rows ?? job.row_count ?? "—";
    const suppressed =
      q.rows_private !== undefined || q.rows_small_n_suppressed !== undefined
        ? sum(q.rows_private, q.rows_small_n_suppressed)
        : q.suppressed_rows ?? q.private_rows ?? q.suppressed ?? 0;
    const missing = q.rows_missing ?? q.missing_rows ?? q.unavailable_rows ?? 0;
    const failed = q.fetches_failed ?? q.failed_requests ?? progress.failed ?? job.failed ?? 0;
    elements.qualityUsable.textContent = formatMetric(usable);
    elements.qualitySuppressed.textContent = formatMetric(suppressed);
    elements.qualityMissing.textContent = formatMetric(missing);
    elements.qualityFailed.textContent = formatMetric(failed);
    elements.quality.hidden = false;
  }

  function formatMetric(value) {
    return typeof value === "number" ? value.toLocaleString() : String(value);
  }

  async function pollJob(id) {
    try {
      const job = await fetchJson(`/api/jobs/${encodeURIComponent(id)}`);
      updateProgress(job);
      const status = String(job.status || "").toLowerCase();
      if (status === "completed" || status === "complete" || status === "succeeded") {
        state.polling = null;
        elements.progressBar.style.width = "100%";
        elements.progressPercent.textContent = "100%";
        elements.jobTitle.textContent = "Your report is ready";
        elements.jobCopy.textContent = "The pull and statistical quality checks are complete.";
        elements.jobBadge.textContent = "Completed";
        elements.jobBadge.className = "state-badge completed";
        readQuality(job);
        configureDownloads(id, job.downloads);
        await loadRows(id);
        updateEstimate();
        return;
      }
      if (status === "failed" || status === "error") {
        state.polling = null;
        elements.jobTitle.textContent = "The report could not be completed";
        elements.jobCopy.textContent = "Review the failure below, adjust the pull if needed, and try again.";
        elements.jobBadge.textContent = "Failed";
        elements.jobBadge.className = "state-badge failed";
        elements.jobFailure.textContent = job.error || job.message || "The job failed without an error message.";
        elements.jobFailure.hidden = false;
        readQuality(job);
        updateEstimate();
        return;
      }
      state.polling = window.setTimeout(() => pollJob(id), 650);
    } catch (error) {
      state.polling = null;
      elements.jobBadge.textContent = "Connection lost";
      elements.jobBadge.className = "state-badge failed";
      elements.jobFailure.textContent = `Could not check report progress: ${error.message}. The local server may have stopped.`;
      elements.jobFailure.hidden = false;
      updateEstimate();
    }
  }

  function configureDownloads(id, downloads = {}) {
    const formats = ["csv", "xlsx", "html", "pdf"];
    formats.forEach((format) => {
      const anchor = $(`#download-${format}`);
      anchor.href = downloads?.[format] || `/api/jobs/${encodeURIComponent(id)}/downloads/${format}`;
    });
    elements.downloadArea.hidden = false;
  }

  async function loadRows(id) {
    try {
      const response = await fetchJson(`/api/jobs/${encodeURIComponent(id)}/rows?limit=500`);
      state.rows = Array.isArray(response) ? response : response.rows || [];
      renderRows();
      elements.resultsSection.hidden = false;
    } catch (error) {
      elements.resultsSection.hidden = true;
      elements.jobFailure.textContent = `The exports are ready, but the table preview could not be loaded: ${error.message}`;
      elements.jobFailure.hidden = false;
    }
  }

  const preferredColumns = [
    "school_name", "district", "district_name", "county", "county_name", "cds_code",
    "year", "indicator", "indicator_category", "student_group", "status", "change",
    "performance", "count", "missing_reason",
  ];

  function labelForColumn(key) {
    const names = {
      cds_code: "CDS code",
      school_name: "School",
      district_name: "District",
      county_name: "County",
      student_group: "Student group",
      indicator_category: "Indicator",
      missing_reason: "Availability",
    };
    return names[key] || key.replaceAll("_", " ").replace(/\b\w/g, (letter) => letter.toUpperCase());
  }

  function displayValue(value) {
    if (value === null || value === undefined || value === "") return { text: "Not available", missing: true };
    if (typeof value === "boolean") return { text: value ? "Yes" : "No", missing: false };
    if (typeof value === "object") return { text: JSON.stringify(value), missing: false };
    return { text: String(value), missing: false };
  }

  function renderRows() {
    const query = elements.tableFilter.value.trim().toLocaleLowerCase();
    const filtered = query
      ? state.rows.filter((row) => Object.values(row).some((value) => String(value ?? "").toLocaleLowerCase().includes(query)))
      : state.rows;
    const allKeys = [...new Set(state.rows.flatMap((row) => Object.keys(row)))];
    const preferred = preferredColumns.filter((key) => allKeys.includes(key));
    const columns = [...preferred, ...allKeys.filter((key) => !preferred.includes(key))].slice(0, 14);

    elements.resultsHead.replaceChildren();
    elements.resultsBody.replaceChildren();
    const headerRow = document.createElement("tr");
    columns.forEach((column) => {
      const th = document.createElement("th");
      th.scope = "col";
      th.textContent = labelForColumn(column);
      headerRow.append(th);
    });
    elements.resultsHead.append(headerRow);

    filtered.slice(0, 500).forEach((row) => {
      const tr = document.createElement("tr");
      columns.forEach((column) => {
        const td = document.createElement("td");
        const displayed = displayValue(row[column]);
        td.textContent = displayed.text;
        if (displayed.missing) td.className = "cell-missing";
        tr.append(td);
      });
      elements.resultsBody.append(tr);
    });

    const visible = Math.min(filtered.length, 500);
    elements.resultCount.textContent = `${visible.toLocaleString()} of ${state.rows.length.toLocaleString()} preview rows shown${query ? " after filtering" : ""}. Exports contain the complete result.`;
  }

  async function startJob(event) {
    event.preventDefault();
    clearGlobalError();
    if ((!state.allSchools && !state.schools.size) || !state.years.size) {
      if (!state.years.size) elements.yearMessage.textContent = "Choose at least one reporting year.";
      showGlobalError("Choose at least one school (or every active school) and one reporting year before starting the pull.");
      return;
    }
    if (state.polling) return;
    resetJobDisplay();
    elements.jobPanel.hidden = false;
    elements.jobPanel.scrollIntoView({ behavior: "smooth", block: "start" });
    elements.submit.disabled = true;
    elements.submit.querySelector("span").textContent = "Starting report…";

    const request = {
      cds_codes: state.allSchools ? [] : [...state.schools.keys()],
      all_schools: state.allSchools,
      years: [...state.years].sort((a, b) => a - b),
      settings: {
        concurrency: numberWithin(elements.concurrency, 1, 64),
        requests_per_second: numberWithin(elements.rate, 1, 1000),
        timeout_seconds: numberWithin(elements.timeout, 1, 120),
      },
    };

    try {
      const job = await fetchJson("/api/jobs", { method: "POST", body: JSON.stringify(request) });
      const id = job.id || job.job_id;
      if (!id) throw new Error("The server did not return a job identifier.");
      state.jobId = id;
      elements.submit.querySelector("span").textContent = "Pull data & build report";
      state.polling = window.setTimeout(() => pollJob(id), 100);
    } catch (error) {
      elements.submit.querySelector("span").textContent = "Pull data & build report";
      elements.jobBadge.textContent = "Not started";
      elements.jobBadge.className = "state-badge failed";
      elements.jobFailure.textContent = `The report could not be started: ${error.message}`;
      elements.jobFailure.hidden = false;
      state.polling = null;
      updateEstimate();
    }
  }

  elements.search.addEventListener("input", scheduleSearch);
  elements.search.addEventListener("focus", () => {
    if (elements.search.value.trim().length >= 2 && state.searchResults.length) renderSearchResults(state.searchResults);
  });
  elements.search.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      if (elements.searchResults.hidden) scheduleSearch();
      else setActiveResult(state.activeResult + 1);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setActiveResult(state.activeResult - 1);
    } else if (event.key === "Enter" && state.activeResult >= 0) {
      event.preventDefault();
      toggleSchool(state.searchResults[state.activeResult]);
    } else if (event.key === "Escape") {
      closeSearchResults();
    }
  });
  document.addEventListener("click", (event) => {
    if (!event.target.closest(".school-combobox")) closeSearchResults();
  });
  elements.clearSchools.addEventListener("click", () => {
    state.schools.clear();
    renderSelectedSchools();
    if (!elements.searchResults.hidden) renderSearchResults(state.searchResults);
    elements.search.focus();
  });
  elements.allSchools.addEventListener("change", () => {
    state.allSchools = elements.allSchools.checked;
    elements.search.disabled = state.allSchools;
    if (state.allSchools) {
      state.schools.clear();
      closeSearchResults();
    }
    renderSelectedSchools();
  });
  elements.form.addEventListener("submit", startJob);
  elements.tableFilter.addEventListener("input", renderRows);

  loadHealth();
  loadYears();
})();
