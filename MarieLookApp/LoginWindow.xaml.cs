using System.ComponentModel;
using System.Windows;
using MarieLookApp.ViewModels;

namespace MarieLookApp;

public partial class LoginWindow : Window
{
    private readonly LoginViewModel _vm;

    public event Action? LoginSuccessful;

    public LoginWindow()
    {
        _vm = new LoginViewModel(App.Supabase);
        DataContext = _vm;

        InitializeComponent();

        _vm.LoginSuccessful += () => LoginSuccessful?.Invoke();
        _vm.CloseRequested += () => Close();
        _vm.PropertyChanged += OnVmPropertyChanged;

        Loaded += (s, e) => EmailBox.Focus();
    }

    private void OnVmPropertyChanged(object? sender, PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(LoginViewModel.IsOtpStep) && _vm.IsOtpStep)
        {
            OtpBox.Focus();
        }
    }
}
