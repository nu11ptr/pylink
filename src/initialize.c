/* Keep CPython's version-dependent configuration structures on the C side. */
#include <Python.h>
#include <stdio.h>
#include <string.h>

static int pybundle_status_error(PyStatus status, char *error, size_t capacity)
{
    if (PyStatus_IsExit(status)) {
        snprintf(error, capacity, "CPython requested exit with code %d", status.exitcode);
    } else {
        snprintf(error, capacity, "%s%s%s",
                 status.func != NULL ? status.func : "",
                 status.func != NULL ? ": " : "",
                 status.err_msg != NULL ? status.err_msg : "unknown CPython initialization error");
    }
    return -1;
}

/* The Rust caller serializes calls and supplies non-null, terminated strings.
 * Return errors to Rust: never invoke Py_ExitStatusException(), which exits the
 * embedding application's process. */
int pybundle_initialize_interpreter(const wchar_t *home, const wchar_t *executable,
                                    char *error, size_t capacity)
{
    PyPreConfig preconfig;
    PyConfig config;
    PyStatus status;
    const char *loaded_version = Py_GetVersion();
    size_t expected_version_length = strlen(PY_VERSION);

    if (Py_IsInitialized()) {
        snprintf(error, capacity,
                 "CPython was initialized outside pybundle; call pybundle::initialize() first");
        return -1;
    }
    if (strncmp(loaded_version, PY_VERSION, expected_version_length) != 0 ||
        loaded_version[expected_version_length] != ' ') {
        snprintf(error, capacity, "loaded CPython does not match the build headers (%s)", PY_VERSION);
        return -1;
    }

    PyPreConfig_InitIsolatedConfig(&preconfig);
    /* Match Rust's UTF-8/surrogateescape conversion of Unix filesystem paths,
     * independently of the embedding application's locale. */
    preconfig.utf8_mode = 1;
    status = Py_PreInitialize(&preconfig);
    if (PyStatus_Exception(status)) {
        return pybundle_status_error(status, error, capacity);
    }

    PyConfig_InitIsolatedConfig(&config);
    config.install_signal_handlers = 0;
    config.parse_argv = 0;
    config.write_bytecode = 0;
    status = PyConfig_SetString(&config, &config.home, home);
    if (PyStatus_Exception(status)) {
        goto error;
    }
    status = PyConfig_SetString(&config, &config.executable, executable);
    if (PyStatus_Exception(status)) {
        goto error;
    }
    status = Py_InitializeFromConfig(&config);
    if (PyStatus_Exception(status)) {
        goto error;
    }
    PyConfig_Clear(&config);

    /* PyO3 (and other embedding APIs) must be able to attach any Rust thread. */
    (void)PyEval_SaveThread();
    return 0;

error:
    /* Copy the message before freeing any configuration-owned storage. */
    pybundle_status_error(status, error, capacity);
    PyConfig_Clear(&config);
    return -1;
}
