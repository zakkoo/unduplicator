/* Inject storage errors at the libc boundary, only for the test's selected path.
 * Loaded solely by regression subprocesses; never linked into the application. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

static int fail(int fd, const char *operation) {
    const char *prefix = getenv("UNDUPLICATOR_TEST_FAULT_PATH");
    const char *op = getenv("UNDUPLICATOR_TEST_FAULT_OP");
    if (!prefix || !op || strcmp(op, operation)) return 0;
    char link[64], path[4096];
    snprintf(link, sizeof(link), "/proc/self/fd/%d", fd);
    ssize_t length = readlink(link, path, sizeof(path) - 1);
    if (length < 0) return 0;
    path[length] = 0;
    if (strncmp(path, prefix, strlen(prefix))) return 0;
    if (getenv("UNDUPLICATOR_TEST_FAULT_EXACT") && strcmp(path, prefix)) return 0;
    const char *after = getenv("UNDUPLICATOR_TEST_FAULT_AFTER");
    if (after) {
        struct stat info;
        if (fstat(fd, &info)) return 0;
        off_t progress = !strcmp(operation, "read") ? lseek(fd, 0, SEEK_CUR) : info.st_size;
        if (progress < atoll(after)) return 0;
    }
    errno = !strcmp(getenv("UNDUPLICATOR_TEST_FAULT_ERRNO"), "ENOSPC") ? ENOSPC : EIO;
    return 1;
}
ssize_t write(int fd, const void *buf, size_t n) {
    ssize_t (*real)(int,const void*,size_t) = dlsym(RTLD_NEXT, "write");
    return fail(fd,"write") ? -1 : real(fd,buf,n);
}
ssize_t read(int fd, void *buf, size_t n) {
    ssize_t (*real)(int,void*,size_t) = dlsym(RTLD_NEXT, "read");
    return fail(fd,"read") ? -1 : real(fd,buf,n);
}
ssize_t pwrite(int fd, const void *buf, size_t n, off_t offset) {
    ssize_t (*real)(int,const void*,size_t,off_t) = dlsym(RTLD_NEXT, "pwrite");
    return fail(fd,"write") ? -1 : real(fd,buf,n,offset);
}
ssize_t pwrite64(int fd, const void *buf, size_t n, off64_t offset) {
    ssize_t (*real)(int,const void*,size_t,off64_t) = dlsym(RTLD_NEXT, "pwrite64");
    return fail(fd,"write") ? -1 : real(fd,buf,n,offset);
}
int fsync(int fd) {
    int (*real)(int) = dlsym(RTLD_NEXT, "fsync");
    return fail(fd,"sync") ? -1 : real(fd);
}
int fdatasync(int fd) {
    int (*real)(int) = dlsym(RTLD_NEXT, "fdatasync");
    return fail(fd,"sync") ? -1 : real(fd);
}
