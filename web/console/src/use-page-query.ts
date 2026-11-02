import { useCallback, useEffect, useState } from "react";
export function usePageQuery() {
  const [search, setSearch] = useState(window.location.search);
  useEffect(() => {
    const pop = () => setSearch(window.location.search);
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);
  const navigate = useCallback((params: URLSearchParams, replace = false) => {
    const query = params.toString();
    const url = `${window.location.pathname}${query ? `?${query}` : ""}`;
    if (replace) window.history.replaceState(null, "", url);
    else window.history.pushState(null, "", url);
    setSearch(window.location.search);
  }, []);
  return { params: new URLSearchParams(search), navigate, search };
}
