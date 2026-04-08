using System;

public class Program {
    public static int Main() {
        int[] values = new int[3];
        values[0] = 10;
        values[1] = 20;
        values[2] = 30;

        try {
            values.GetValue(new int[0]);
            return 1;
        } catch (ArgumentException) {
            return 42;
        } catch (Exception) {
            return 2;
        }
    }
}
