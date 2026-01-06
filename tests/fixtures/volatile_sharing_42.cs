public class Program {
    public static volatile int Flag = 0;
    public static int Data = 0;

    public static int Main() {
        if (Flag == 0) {
            Data = 42;
            Flag = 1;
        } else {
            while (Flag != 1) { }
            if (Data != 42) return 1;
        }
        return 42;
    }
}
