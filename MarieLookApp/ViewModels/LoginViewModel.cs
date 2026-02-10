using System.Windows.Input;
using MarieLookApp.Services;

namespace MarieLookApp.ViewModels;

public class LoginViewModel : ViewModelBase
{
    private readonly SupabaseService _supabase;

    private string _email = string.Empty;
    private string _otp = string.Empty;
    private string _statusText = string.Empty;
    private bool _isStatusError;
    private bool _isEmailStep = true;
    private bool _isOtpStep;
    private bool _isBusy;

    public LoginViewModel(SupabaseService supabase)
    {
        _supabase = supabase;

        SendOtpCommand = new AsyncRelayCommand(SendOtpAsync, () => !IsBusy);
        VerifyCommand = new AsyncRelayCommand(VerifyAsync, () => !IsBusy);
        BackCommand = new RelayCommand(GoBack);
    }

    public string Email
    {
        get => _email;
        set => SetProperty(ref _email, value);
    }

    public string Otp
    {
        get => _otp;
        set => SetProperty(ref _otp, value);
    }

    public string StatusText
    {
        get => _statusText;
        set => SetProperty(ref _statusText, value);
    }

    public bool IsStatusError
    {
        get => _isStatusError;
        set => SetProperty(ref _isStatusError, value);
    }

    public bool IsEmailStep
    {
        get => _isEmailStep;
        set => SetProperty(ref _isEmailStep, value);
    }

    public bool IsOtpStep
    {
        get => _isOtpStep;
        set => SetProperty(ref _isOtpStep, value);
    }

    public bool IsBusy
    {
        get => _isBusy;
        set
        {
            if (SetProperty(ref _isBusy, value))
            {
                (SendOtpCommand as AsyncRelayCommand)?.RaiseCanExecuteChanged();
                (VerifyCommand as AsyncRelayCommand)?.RaiseCanExecuteChanged();
            }
        }
    }

    public ICommand SendOtpCommand { get; }
    public ICommand VerifyCommand { get; }
    public ICommand BackCommand { get; }

    public event Action? LoginSuccessful;
    public event Action? CloseRequested;

    private async Task SendOtpAsync()
    {
        var email = Email.Trim();

        if (string.IsNullOrEmpty(email) || !email.Contains('@'))
        {
            IsStatusError = true;
            StatusText = "Please enter a valid email address.";
            return;
        }

        IsBusy = true;
        IsStatusError = false;
        StatusText = "Sending code...";

        var result = await _supabase.SendOtpAsync(email);

        if (result.Success)
        {
            IsEmailStep = false;
            IsOtpStep = true;
            StatusText = string.Empty;
        }
        else
        {
            IsStatusError = true;
            StatusText = result.Error ?? "Failed to send code.";
        }

        IsBusy = false;
    }

    private async Task VerifyAsync()
    {
        var otp = Otp.Trim();

        if (string.IsNullOrEmpty(otp))
        {
            IsStatusError = true;
            StatusText = "Please enter the code.";
            return;
        }

        IsBusy = true;
        IsStatusError = false;
        StatusText = "Verifying...";

        var result = await _supabase.VerifyOtpAsync(Email.Trim(), otp);

        if (result.Success)
        {
            LoginSuccessful?.Invoke();
            CloseRequested?.Invoke();
        }
        else
        {
            IsStatusError = true;
            StatusText = result.Error ?? "Invalid code.";
        }

        IsBusy = false;
    }

    private void GoBack()
    {
        IsOtpStep = false;
        IsEmailStep = true;
        Otp = string.Empty;
        StatusText = string.Empty;
        IsStatusError = false;
    }
}
