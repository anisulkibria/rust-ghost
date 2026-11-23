/* react/jsx-runtime for the UMD build of React.
 *
 * koenig-lexical is compiled against the automatic JSX transform, so it calls
 * jsx()/jsxs() rather than createElement(). Its UMD wrapper resolves both
 * "react" and "react/jsx-runtime" to window.React — but React's own UMD build
 * predates the new transform and only ships createElement, so those calls are
 * missing and the editor throws on its first render.
 *
 * The two are equivalent: jsx(type, props, key) is createElement with the
 * children lifted out of props and the key passed separately. React 19 drops
 * UMD builds altogether, which is why this is a shim rather than a newer
 * React.
 */
(function () {
  var React = window.React;
  if (!React || typeof React.jsx === 'function') { return; }

  function jsx(type, props, key) {
    var config = {};
    var hasChildren = false;
    var children;
    for (var name in props) {
      if (!Object.prototype.hasOwnProperty.call(props, name)) { continue; }
      if (name === 'children') { children = props[name]; hasChildren = true; continue; }
      config[name] = props[name];
    }
    if (key !== undefined) { config.key = key; }
    if (!hasChildren) { return React.createElement(type, config); }
    // Several children are passed as separate arguments, not as one array:
    // createElement then treats them as static, the way the compiler meant
    // them, instead of a dynamic list that would want keys of its own.
    if (Array.isArray(children)) { return React.createElement.apply(null, [type, config].concat(children)); }
    return React.createElement(type, config, children);
  }

  React.jsx = jsx;
  React.jsxs = jsx;
  React.jsxDEV = jsx;
  // The runtime re-exports Fragment, which React already has under the same
  // name; defining it only if missing keeps this from shadowing the real one.
  if (!React.Fragment) { React.Fragment = React.createElement; }
})();
