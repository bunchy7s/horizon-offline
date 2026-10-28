/* A stand-in for the Windows SDK's fxc.exe, so gpui's Windows shaders can be compiled while
 * cross-compiling the launcher on Linux: it parses fxc's arguments, compiles the HLSL with
 * D3DCompile (d3dcompiler_47, run under Wine) and writes the /Fh header gpui's build script
 * reads ("const BYTE <name>[] = {0x..,...};").
 *
 * Build: x86_64-w64-mingw32-gcc tools/fxc_shim.c -o tools/fxc_shim.exe -ld3dcompiler_47
 * Run:   wine tools/fxc_shim.exe /T vs_4_1 /E main /Fh out.h /Vn name /O3 shader.hlsl
 */
#include <windows.h>
#include <d3dcompiler.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>

static char *read_file(const char *path, size_t *size) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *data = malloc(n + 1);
    if (fread(data, 1, n, f) != (size_t)n) { fclose(f); free(data); return NULL; }
    fclose(f);
    data[n] = 0;
    *size = n;
    return data;
}

/* An ID3DInclude that opens #include files relative to the shader's folder (vkd3d's D3DCompile
 * under Wine has no standard include handler). Includes are flat in gpui's shaders. */
static char include_dir[MAX_PATH] = ".";

static HRESULT STDMETHODCALLTYPE include_open(ID3DInclude *self, D3D_INCLUDE_TYPE type,
                                              const char *file, const void *parent,
                                              const void **data, unsigned int *bytes) {
    (void)self; (void)type; (void)parent;
    char path[MAX_PATH];
    if (file[0] == '/' || (file[1] == ':' && file[2] == '\\')) {
        snprintf(path, sizeof(path), "%s", file);
    } else {
        snprintf(path, sizeof(path), "%s/%s", include_dir, file);
    }
    size_t size = 0;
    char *text = read_file(path, &size);
    if (!text) return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);
    *data = text;
    *bytes = (unsigned int)size;
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE include_close(ID3DInclude *self, const void *data) {
    (void)self;
    free((void *)data);
    return S_OK;
}

static ID3DIncludeVtbl include_vtbl_struct = { include_open, include_close };
static struct {
    ID3DIncludeVtbl *lpVtbl;
} include_instance = { &include_vtbl_struct };
#define include_vtbl ((ID3DInclude *)&include_instance)

int main(int argc, char **argv) {
    const char *target = NULL, *entry = NULL, *header = NULL, *name = NULL, *source = NULL;
    for (int i = 1; i < argc; i++) {
        /* Only the flags gpui passes are flags; anything else (a Unix path starts with
         * '/' too, under Wine) is the shader. */
        const char *flag = argv[i];
        int takes_value = !strcmp(flag, "/T") || !strcmp(flag, "/E") ||
                          !strcmp(flag, "/Fh") || !strcmp(flag, "/Vn") || !strcmp(flag, "/Fo");
        int no_value = !strcmp(flag, "/O3") || !strcmp(flag, "/O2") ||
                       !strcmp(flag, "/O1") || !strcmp(flag, "/O0");
        if (!takes_value && !no_value) {
            source = argv[i];
            continue;
        }
        if (no_value || i + 1 >= argc) {
            continue;
        }
        const char *value = argv[++i];
        if (!strcmp(flag, "/T")) target = value;
        else if (!strcmp(flag, "/E")) entry = value;
        else if (!strcmp(flag, "/Fh")) header = value;
        else if (!strcmp(flag, "/Vn")) name = value;
        /* /Fo: ignored, only the header is read. */
    }
    if (!target || !entry || !header || !name || !source) {
        fprintf(stderr, "fxc_shim: need /T /E /Fh /Vn and the shader path (argc=%d)\n", argc);
        for (int i = 0; i < argc; i++) fprintf(stderr, "  %d:[%s]\n", i, argv[i]);
        return 1;
    }
    size_t size = 0;
    char *text = read_file(source, &size);
    if (!text) { fprintf(stderr, "fxc_shim: cannot read %s\n", source); return 1; }

    /* Includes resolve relative to the shader's folder. */
    {
        const char *slash = strrchr(source, '/');
        size_t n = slash ? (size_t)(slash - source) : 1;
        if (n >= sizeof(include_dir)) n = sizeof(include_dir) - 1;
        memcpy(include_dir, source, n);
        include_dir[n] = 0;
    }

    ID3DBlob *code = NULL, *errors = NULL;
    HRESULT hr = D3DCompile(text, size, source, NULL, include_vtbl, entry, target,
                            D3DCOMPILE_ENABLE_STRICTNESS, 0, &code, &errors);
    if (errors) {
        fprintf(stderr, "%.*s", (int)errors->lpVtbl->GetBufferSize(errors),
                (const char *)errors->lpVtbl->GetBufferPointer(errors));
        errors->lpVtbl->Release(errors);
    }
    if (FAILED(hr)) { fprintf(stderr, "fxc_shim: D3DCompile failed (0x%08lx)\n", (long)hr); return 1; }

    FILE *out = fopen(header, "w");
    if (!out) { fprintf(stderr, "fxc_shim: cannot write %s\n", header); return 1; }
    fprintf(out, "#if 0\n#endif\nconst BYTE %s[] =\n{", name);
    const unsigned char *bytes = (const unsigned char *)code->lpVtbl->GetBufferPointer(code);
    SIZE_T n = code->lpVtbl->GetBufferSize(code);
    for (SIZE_T i = 0; i < n; i++) {
        fprintf(out, "%s0x%02x", (i % 16 == 0) ? "\n    " : " ", bytes[i]);
        if (i + 1 < n) fprintf(out, ",");
    }
    fprintf(out, "\n};\n");
    fclose(out);
    code->lpVtbl->Release(code);
    free(text);
    return 0;
}
