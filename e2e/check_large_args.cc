#include <cstring>
#include <fstream>

int main(int argc, char** argv) {
    if (argc != 40 || std::strlen(argv[1]) != 4096) return 1;
    for (int i = 0; i < 4096; ++i) if (argv[1][i] != 'x') return 2;
    for (int i = 2; i < 39; ++i) if (std::strcmp(argv[i], "literal")) return 3;
    // Index 39 must have been resolved through runfiles by the large stub.
    return std::ifstream(argv[39]).good() ? 0 : 4;
}
