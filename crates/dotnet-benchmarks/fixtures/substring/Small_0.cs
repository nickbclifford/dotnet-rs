using System;
public class Program {
    public static int Main() {
        string source = "abcdefghijklmnopqrstuvwxyz";
        int total = 0;
        for (int i = 0; i < 20000; i++) {
            total += source.Substring(i % 10, 0).Length;
            total += source.Substring(i % 10, 8).Length;
        }
        return total == 160000 ? 0 : 1;
    }
}
