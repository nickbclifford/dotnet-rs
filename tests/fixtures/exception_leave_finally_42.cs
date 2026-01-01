using System;

public class Program {
    static int result = 0;
    public static int Main() {
        try {
            try {
                throw new Exception();
            } catch (Exception) {
                result = 21;
                return 42; // This return should be intercepted by finally
            } finally {
                result += 21;
            }
        } catch (Exception) {
            result = 100; // Should not be reached
        }
        
        return result;
    }
}
