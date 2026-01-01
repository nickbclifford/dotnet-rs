using System;

public class Program {
    static int result = 0;
    public static int Main() {
        try {
            try {
                throw new Exception();
            } catch (Exception) {
                result = 21;
                throw;
            }
        } catch (Exception) {
            result += 21;
        }
        return result;
    }
}
