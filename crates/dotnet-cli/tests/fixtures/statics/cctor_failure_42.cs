using System;

public class ThrowsCctor
{
    public static int x;
    static ThrowsCctor()
    {
        throw new InvalidOperationException("cctor failed");
    }
}

public class Program
{
    public static int Main()
    {
        try
        {
            int val = ThrowsCctor.x;
            return 1; // Should not reach here
        }
        catch (TypeInitializationException)
        {
            // Expected
            try
            {
                int val2 = ThrowsCctor.x;
                return 2; // Should not reach here
            }
            catch (TypeInitializationException)
            {
                return 42; // Success
            }
            catch (Exception)
            {
                return 3; // Wrong exception type on second access
            }
        }
        catch (InvalidOperationException)
        {
            return 4; // First access threw unwrapped exception
        }
        catch (Exception)
        {
            return 5; // Some other exception
        }
    }
}
