import type { WeatherInfo } from "../weather";

type Props = {
  weather: WeatherInfo;
};

export default function WeatherWidget({ weather }: Props) {
  return (
    <section className="widget weather-widget">
      <header className="widget-head">
        <span className="widget-title">天气</span>
        <span className="widget-sub">{weather.city}</span>
      </header>

      <div className="weather-main">
        {weather.iconUrl ? (
          <img className="weather-big-icon" src={weather.iconUrl} alt="" />
        ) : (
          <span className="weather-big-icon" aria-hidden>
            ☁️
          </span>
        )}
        <div className="weather-main-text">
          <p className="weather-temp-lg">{weather.temp}°</p>
          <p className="weather-condition">{weather.condition}</p>
        </div>
      </div>

      <ul className="weather-meta">
        <li>
          <span>体感</span>
          <strong>{weather.feelsLike}°</strong>
        </li>
        <li>
          <span>湿度</span>
          <strong>{weather.humidity}%</strong>
        </li>
        <li>
          <span>风力</span>
          <strong>{weather.wind}</strong>
        </li>
        <li>
          <span>高低</span>
          <strong>
            {weather.high}° / {weather.low}°
          </strong>
        </li>
      </ul>
    </section>
  );
}
