public class Program {
    public static int Main() {
        int[] arr = new int[5];
        if (arr.Length != 5) return 1;
        arr[0] = 42;
        if (arr[0] != 42) return 2;
        arr[4] = 123;
        if (arr[4] != 123) return 3;
        return 0;
    }
}
