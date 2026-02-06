public class Program {
    static void SetValue(ref int x) { x = 42; }
    public static int Main() {
        int val = 0;
        SetValue(ref val);
        return val == 42 ? 0 : 1;
    }
}
